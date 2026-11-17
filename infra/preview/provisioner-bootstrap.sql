-- Run as a trusted superuser on database thrust. Supply the password and shared connection
-- limit through the environment; never pass them as psql arguments. This transaction installs
-- a private registry, so pre-existing pr_* objects are deliberately not adopted.
\set ON_ERROR_STOP on
\getenv provisioner_password THRUST_PREVIEW_PROVISIONER_PASSWORD
\getenv pr_limit THRUST_PR_ROLE_CONNECTION_LIMIT
BEGIN;
DO $$ BEGIN
  IF current_database() <> 'thrust' OR NOT (SELECT rolsuper FROM pg_catalog.pg_roles WHERE rolname = current_user) THEN
    RAISE EXCEPTION 'bootstrap requires a superuser on database thrust';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_catalog.pg_database d, pg_catalog.aclexplode(COALESCE(d.datacl, pg_catalog.acldefault('d', d.datdba))) a
    WHERE d.datname = current_database() AND a.grantee = 0 AND a.privilege_type = 'CREATE') THEN
    RAISE EXCEPTION 'revoke database CREATE from PUBLIC before bootstrap';
  END IF;
END $$;
SELECT (:'provisioner_password' ~ '^[A-Za-z0-9_-]{24,128}$' AND :'pr_limit' ~ '^([5-9]|[1-9][0-9]|100)$') AS valid_input \gset
\if :valid_input
\else
\echo 'invalid bootstrap password or connection limit'
\quit 3
\endif
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'thrust_preview_provisioner') THEN
    RAISE EXCEPTION 'provisioner role already exists; investigate before bootstrap';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname = 'preview_admin') THEN
    RAISE EXCEPTION 'preview_admin schema already exists; investigate before bootstrap';
  END IF;
END $$;
CREATE ROLE thrust_preview_provisioner LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOINHERIT;
SELECT pg_catalog.format('ALTER ROLE thrust_preview_provisioner PASSWORD %L', :'provisioner_password') \gexec
GRANT CONNECT ON DATABASE thrust TO thrust_preview_provisioner;
CREATE SCHEMA preview_admin;
REVOKE ALL ON SCHEMA preview_admin FROM PUBLIC;
GRANT USAGE ON SCHEMA preview_admin TO thrust_preview_provisioner;
CREATE TABLE preview_admin.managed (number integer PRIMARY KEY, role_oid oid NOT NULL, schema_oid oid NOT NULL, pending boolean NOT NULL DEFAULT true);
CREATE TABLE preview_admin.settings (connection_limit integer NOT NULL);
INSERT INTO preview_admin.settings VALUES (:'pr_limit'::integer);
REVOKE ALL ON ALL TABLES IN SCHEMA preview_admin FROM PUBLIC, thrust_preview_provisioner;

CREATE FUNCTION preview_admin.provision(n integer, password text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE name text; r oid; s oid; entry preview_admin.managed%ROWTYPE; lim integer;
BEGIN
  IF n IS NULL OR n < 1 OR n > 999999999 OR password IS NULL OR password !~ '^[A-Za-z0-9_-]{24,128}$' THEN
    RAISE EXCEPTION 'invalid preview request' USING ERRCODE = '22023';
  END IF;
  LOCK TABLE preview_admin.managed IN EXCLUSIVE MODE;
  name := 'pr_' || n::text;
  SELECT oid INTO r FROM pg_catalog.pg_roles WHERE rolname = name;
  SELECT oid INTO s FROM pg_catalog.pg_namespace WHERE nspname = name;
  SELECT * INTO entry FROM preview_admin.managed WHERE number = n;
  IF FOUND THEN
    IF r IS DISTINCT FROM entry.role_oid OR s IS DISTINCT FROM entry.schema_oid OR EXISTS
      (SELECT 1 FROM pg_catalog.pg_roles WHERE oid = r AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
      OR EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members WHERE member = r OR roleid = r)
      OR NOT EXISTS (SELECT 1 FROM pg_catalog.pg_namespace WHERE oid = s AND nspowner = r) THEN
      RAISE EXCEPTION 'preview resource ownership changed' USING ERRCODE = '55000';
    END IF;
  ELSE
    IF r IS NOT NULL OR s IS NOT NULL THEN RAISE EXCEPTION 'unmanaged preview resource collision' USING ERRCODE = '55000'; END IF;
    EXECUTE pg_catalog.format('CREATE ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS', name);
    EXECUTE pg_catalog.format('CREATE SCHEMA %I AUTHORIZATION %I', name, name);
    SELECT oid INTO r FROM pg_catalog.pg_roles WHERE rolname = name;
    SELECT oid INTO s FROM pg_catalog.pg_namespace WHERE nspname = name;
    INSERT INTO preview_admin.managed (number, role_oid, schema_oid) VALUES (n, r, s);
  END IF;
  SELECT connection_limit INTO STRICT lim FROM preview_admin.settings;
  EXECUTE pg_catalog.format('ALTER ROLE %I LOGIN PASSWORD %L CONNECTION LIMIT %s', name, password, lim);
  EXECUTE pg_catalog.format('ALTER ROLE %I IN DATABASE %I SET search_path = %I', name, current_database(), name);
  UPDATE preview_admin.managed SET pending = true WHERE number = n;
END $$;

-- Called only after the matching Hyperdrive configuration accepts the same password. If the
-- caller crashes before this point, inventory keeps the resource eligible for retry.
CREATE FUNCTION preview_admin.acknowledge(n integer) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
  IF n IS NULL OR n < 1 OR n > 999999999 THEN RAISE EXCEPTION 'invalid preview request' USING ERRCODE = '22023'; END IF;
  UPDATE preview_admin.managed SET pending = false WHERE number = n;
  IF NOT FOUND THEN RAISE EXCEPTION 'preview is not managed' USING ERRCODE = '55000'; END IF;
END $$;

CREATE FUNCTION preview_admin.deprovision(n integer) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE name text; r oid; s oid; entry preview_admin.managed%ROWTYPE;
BEGIN
  IF n IS NULL OR n < 1 OR n > 999999999 THEN RAISE EXCEPTION 'invalid preview request' USING ERRCODE = '22023'; END IF;
  LOCK TABLE preview_admin.managed IN EXCLUSIVE MODE;
  name := 'pr_' || n::text;
  SELECT oid INTO r FROM pg_catalog.pg_roles WHERE rolname = name;
  SELECT oid INTO s FROM pg_catalog.pg_namespace WHERE nspname = name;
  SELECT * INTO entry FROM preview_admin.managed WHERE number = n;
  IF NOT FOUND THEN
    IF r IS NOT NULL OR s IS NOT NULL THEN RAISE EXCEPTION 'unmanaged preview resource collision' USING ERRCODE = '55000'; END IF;
    RETURN;
  END IF;
  IF r IS DISTINCT FROM entry.role_oid OR s IS DISTINCT FROM entry.schema_oid OR EXISTS
    (SELECT 1 FROM pg_catalog.pg_roles WHERE oid = r AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
    OR EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members WHERE member = r OR roleid = r)
    OR NOT EXISTS (SELECT 1 FROM pg_catalog.pg_namespace WHERE oid = s AND nspowner = r) THEN
    RAISE EXCEPTION 'preview resource ownership changed' USING ERRCODE = '55000';
  END IF;
  PERFORM pg_catalog.pg_terminate_backend(pid) FROM pg_catalog.pg_stat_activity WHERE usesysid = r AND pid <> pg_catalog.pg_backend_pid();
  EXECUTE pg_catalog.format('DROP SCHEMA %I CASCADE', name);
  EXECUTE pg_catalog.format('DROP ROLE %I', name);
  DELETE FROM preview_admin.managed WHERE number = n;
END $$;

CREATE FUNCTION preview_admin.inventory() RETURNS TABLE(number integer, schema_exists boolean, login_role boolean, search_path boolean, pending boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_catalog.pg_roles r WHERE r.rolname ~ '^pr_[0-9]+$' AND NOT EXISTS
      (SELECT 1 FROM preview_admin.managed m WHERE m.role_oid = r.oid AND r.rolname = 'pr_' || m.number))
    OR EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n WHERE n.nspname ~ '^pr_[0-9]+$' AND NOT EXISTS
      (SELECT 1 FROM preview_admin.managed m WHERE m.schema_oid = n.oid AND n.nspname = 'pr_' || m.number)) THEN
    RAISE EXCEPTION 'unmanaged preview resources require operator reconciliation' USING ERRCODE = '55000';
  END IF;
  RETURN QUERY SELECT m.number,
    EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n WHERE n.oid = m.schema_oid AND n.nspname = 'pr_' || m.number AND n.nspowner = m.role_oid),
    EXISTS (SELECT 1 FROM pg_catalog.pg_roles r WHERE r.oid = m.role_oid AND r.rolname = 'pr_' || m.number AND r.rolcanlogin),
    EXISTS (SELECT 1 FROM pg_catalog.pg_db_role_setting d WHERE d.setrole = m.role_oid AND d.setdatabase = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database()) AND ('search_path=pr_' || m.number) = ANY(d.setconfig)), m.pending
  FROM preview_admin.managed m ORDER BY m.number;
END
$$;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA preview_admin FROM PUBLIC;
GRANT EXECUTE ON FUNCTION preview_admin.provision(integer, text), preview_admin.deprovision(integer), preview_admin.acknowledge(integer), preview_admin.inventory() TO thrust_preview_provisioner;
COMMIT;
