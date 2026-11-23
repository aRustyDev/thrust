//! Validation for the internal preview provisioner. Access is enforced at the Cloudflare edge.
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// An immutable plan for one preview's disposable database cluster.
///
/// A plan deliberately identifies two different things: a base snapshot, which may be reused
/// when its trusted migration inputs are byte-for-byte identical, and a live cluster, which is
/// unique to the PR. A caller must never use `base_snapshot` as a live database identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewClusterPlan {
    pub number: i32,
    pub fingerprint: String,
    pub base_snapshot: String,
    pub cluster: String,
    pub idempotency_key: String,
}

/// Persisted phase of one PR's preview-cluster lease.
///
/// The future Kubernetes-side controller stores this alongside the non-secret resource receipt.
/// It must not infer a phase from resource names: an interrupted operation is indistinguishable
/// from a completed one without durable lease state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterLeasePhase {
    Provisioning,
    Ready,
    Deleting,
}

/// A PR-local lease for the named mutable cluster. Shared base snapshots are not leased here and
/// therefore cannot be deleted as a side effect of closing one PR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewClusterLease {
    pub plan: PreviewClusterPlan,
    pub phase: ClusterLeasePhase,
}

impl PreviewClusterLease {
    pub fn provisioning(plan: PreviewClusterPlan) -> Self {
        Self { plan, phase: ClusterLeasePhase::Provisioning }
    }

    pub fn request_deletion(&mut self) {
        self.phase = ClusterLeasePhase::Deleting;
    }

    /// Produces a candidate lease for a changed trusted fingerprint.
    ///
    /// A controller keeps the current ready lease attached while it provisions this candidate,
    /// then switches the active receipt atomically. It must reject a replacement request while
    /// deletion is in progress.
    pub fn replacement_candidate(&self, plan: PreviewClusterPlan) -> Option<Self> {
        if self.phase == ClusterLeasePhase::Deleting || self.plan == plan {
            return None;
        }
        Some(Self::provisioning(plan))
    }
}

/// The non-secret resource facts checked on every controller retry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClusterLeaseInventory {
    pub snapshot: bool,
    pub cluster: bool,
    pub connectivity: bool,
}

/// Why inventory cannot be safely reconciled automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterLeaseDrift {
    ClusterWithoutSnapshot,
    ConnectivityWithoutCluster,
    ReadyLeaseIsIncomplete,
}

/// One idempotent controller action. The executor records the outcome before the next call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClusterLeaseAction {
    EnsureSnapshot { snapshot: String },
    EnsureCluster { cluster: String },
    EnsureConnectivity { cluster: String },
    MarkReady,
    DeleteConnectivity { cluster: String },
    DeleteCluster { cluster: String },
    DeleteLease,
    Refuse(ClusterLeaseDrift),
    Noop,
}

/// Selects at most one safe action for a lease retry.
///
/// Provisioning advances snapshot -> cluster -> connectivity -> durable ready receipt. A partial
/// inventory is never repaired by guessing: the controller must inspect the resource receipt and
/// recover deliberately. Deletion reverses the mutable dependency order and intentionally leaves
/// the fingerprint-keyed snapshot for other active leases or the snapshot garbage collector.
pub fn reconcile_cluster_lease(lease: &PreviewClusterLease, inventory: ClusterLeaseInventory) -> ClusterLeaseAction {
    if lease.phase == ClusterLeasePhase::Deleting {
        if inventory.connectivity {
            return ClusterLeaseAction::DeleteConnectivity { cluster: lease.plan.cluster.clone() };
        }
        if inventory.cluster {
            return ClusterLeaseAction::DeleteCluster { cluster: lease.plan.cluster.clone() };
        }
        return ClusterLeaseAction::DeleteLease;
    }

    if inventory.connectivity && !inventory.cluster {
        return ClusterLeaseAction::Refuse(ClusterLeaseDrift::ConnectivityWithoutCluster);
    }
    if inventory.cluster && !inventory.snapshot {
        return ClusterLeaseAction::Refuse(ClusterLeaseDrift::ClusterWithoutSnapshot);
    }

    if lease.phase == ClusterLeasePhase::Ready {
        return if inventory.snapshot && inventory.cluster && inventory.connectivity {
            ClusterLeaseAction::Noop
        } else {
            ClusterLeaseAction::Refuse(ClusterLeaseDrift::ReadyLeaseIsIncomplete)
        };
    }

    if !inventory.snapshot {
        return ClusterLeaseAction::EnsureSnapshot { snapshot: lease.plan.base_snapshot.clone() };
    }
    if !inventory.cluster {
        return ClusterLeaseAction::EnsureCluster { cluster: lease.plan.cluster.clone() };
    }
    if !inventory.connectivity {
        return ClusterLeaseAction::EnsureConnectivity { cluster: lease.plan.cluster.clone() };
    }
    ClusterLeaseAction::MarkReady
}

/// Computes the names a Kubernetes-side preview-cluster controller must use.
///
/// `migration_artifact_sha256` is the SHA-256 of the exact, independently authorized migration
/// artifact, not a user-supplied schema name. `bootstrap_sha256` identifies the immutable
/// database bootstrap image/configuration. The controller owns resource quotas and credentials;
/// neither can be selected by this request.
pub fn preview_cluster_plan(
    number: i32,
    migration_artifact_sha256: &str,
    postgres_major: u16,
    bootstrap_sha256: &str,
) -> Option<PreviewClusterPlan> {
    if !(1..=999_999_999).contains(&number) || !(14..=18).contains(&postgres_major) {
        return None;
    }
    let migration = parse_sha256(migration_artifact_sha256)?;
    let bootstrap = parse_sha256(bootstrap_sha256)?;

    // Tagged fixed-width bytes avoid the ambiguity of string concatenation and leave room for
    // a deliberately versioned next identity format.
    let mut digest = Sha256::new();
    digest.update(b"thrust-preview-snapshot/v1\0");
    digest.update(postgres_major.to_be_bytes());
    digest.update(migration);
    digest.update(bootstrap);
    let fingerprint = hex(&digest.finalize());
    let snapshot_suffix = fingerprint.get(..16)?;
    let cluster_suffix = fingerprint.get(..12)?;
    let base_snapshot = format!("thrust-preview-base-pg{postgres_major}-{snapshot_suffix}");
    let cluster = format!("thrust-preview-pr-{number}-{cluster_suffix}");
    let idempotency_key = format!("preview/{number}/{fingerprint}");
    Some(PreviewClusterPlan { number, fingerprint, base_snapshot, cluster, idempotency_key })
}

fn parse_sha256(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return None;
    }
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return None;
    }
    let mut parsed = [0_u8; 32];
    for (slot, pair) in parsed.iter_mut().zip(pairs) {
        *slot = (hex_value(*pair.first()?)? << 4) | hex_value(*pair.get(1)?)?;
    }
    Some(parsed)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        for nibble in [byte >> 4, byte & 0x0f] {
            if let Some(character) = HEX.get(usize::from(nibble)) {
                value.push(char::from(*character));
            }
        }
    }
    value
}

#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    Inventory,
    Provision(i32),
    Deprovision(i32),
    Acknowledge(i32),
}

pub fn route(method: &str, path: &str) -> Option<Route> {
    if method == "GET" && path == "/v1/previews" {
        return Some(Route::Inventory);
    }
    let number = path.strip_prefix("/v1/previews/")?;
    if number.is_empty() || number.len() > 9 || number.starts_with('0') || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number = number.parse().ok()?;
    match method {
        "PUT" => Some(Route::Provision(number)),
        "DELETE" => Some(Route::Deprovision(number)),
        "POST" => Some(Route::Acknowledge(number)),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    password: String,
}

pub fn password(bytes: &[u8]) -> Option<String> {
    if bytes.len() > 256 {
        return None;
    }
    let body: Body = serde_json::from_slice(bytes).ok()?;
    if !(24..=128).contains(&body.password.len())
        || !body.password.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    Some(body.password)
}

#[cfg(target_arch = "wasm32")]
mod runtime;

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use proptest::strategy::Strategy;

    fn digest_strategy() -> impl Strategy<Value = String> {
        proptest::collection::vec(prop::sample::select(b"0123456789abcdef".to_vec()), 64)
            .prop_map(|bytes| bytes.into_iter().map(char::from).collect())
    }

    fn lease(number: i32, phase: ClusterLeasePhase) -> PreviewClusterLease {
        PreviewClusterLease {
            plan: PreviewClusterPlan {
                number,
                fingerprint: "f".repeat(64),
                base_snapshot: "thrust-preview-base-pg16-ffffffffffffffff".to_owned(),
                cluster: format!("thrust-preview-pr-{number}-ffffffffffff"),
                idempotency_key: format!("preview/{number}/{}", "f".repeat(64)),
            },
            phase,
        }
    }

    fn model_action(phase: ClusterLeasePhase, inventory: ClusterLeaseInventory) -> &'static str {
        if phase == ClusterLeasePhase::Deleting {
            return if inventory.connectivity {
                "delete-connectivity"
            } else if inventory.cluster {
                "delete-cluster"
            } else {
                "delete-lease"
            };
        }
        if inventory.connectivity && !inventory.cluster {
            return "refuse-connectivity-without-cluster";
        }
        if inventory.cluster && !inventory.snapshot {
            return "refuse-cluster-without-snapshot";
        }
        if phase == ClusterLeasePhase::Ready {
            return if inventory.snapshot && inventory.cluster && inventory.connectivity {
                "noop"
            } else {
                "refuse-ready-incomplete"
            };
        }
        if !inventory.snapshot {
            "ensure-snapshot"
        } else if !inventory.cluster {
            "ensure-cluster"
        } else if !inventory.connectivity {
            "ensure-connectivity"
        } else {
            "mark-ready"
        }
    }

    fn action_name(action: &ClusterLeaseAction) -> &'static str {
        match action {
            ClusterLeaseAction::EnsureSnapshot { .. } => "ensure-snapshot",
            ClusterLeaseAction::EnsureCluster { .. } => "ensure-cluster",
            ClusterLeaseAction::EnsureConnectivity { .. } => "ensure-connectivity",
            ClusterLeaseAction::MarkReady => "mark-ready",
            ClusterLeaseAction::DeleteConnectivity { .. } => "delete-connectivity",
            ClusterLeaseAction::DeleteCluster { .. } => "delete-cluster",
            ClusterLeaseAction::DeleteLease => "delete-lease",
            ClusterLeaseAction::Refuse(ClusterLeaseDrift::ClusterWithoutSnapshot) => "refuse-cluster-without-snapshot",
            ClusterLeaseAction::Refuse(ClusterLeaseDrift::ConnectivityWithoutCluster) => {
                "refuse-connectivity-without-cluster"
            }
            ClusterLeaseAction::Refuse(ClusterLeaseDrift::ReadyLeaseIsIncomplete) => "refuse-ready-incomplete",
            ClusterLeaseAction::Noop => "noop",
        }
    }

    proptest! {
        #[test]
        fn prop_valid_numbers_route(n in 1i32..=999_999_999) {
            prop_assert_eq!(route("PUT", &format!("/v1/previews/{n}")), Some(Route::Provision(n)));
            prop_assert_eq!(route("DELETE", &format!("/v1/previews/{n}")), Some(Route::Deprovision(n)));
        }
        #[test]
        fn prop_untrusted_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..1024)) {
            let _ = password(&bytes);
        }
        #[test]
        fn prop_valid_password_round_trip(p in "[A-Za-z0-9_-]{24,128}") {
            let body = serde_json::json!({"password": p});
            prop_assert_eq!(password(body.to_string().as_bytes()), body["password"].as_str().map(str::to_owned));
        }
        #[test]
        fn prop_snapshot_is_reusable_but_live_cluster_is_unique(
            artifact in digest_strategy(),
            bootstrap in digest_strategy(),
            postgres_major in 14_u16..=18,
            first in 1_i32..=999_999_998,
        ) {
            let second = first + 1;
            let left = preview_cluster_plan(first, &artifact, postgres_major, &bootstrap);
            let right = preview_cluster_plan(second, &artifact, postgres_major, &bootstrap);
            match (left, right) {
                (Some(left), Some(right)) => {
                    prop_assert_eq!(left.base_snapshot, right.base_snapshot);
                    prop_assert_ne!(left.cluster, right.cluster);
                    prop_assert_ne!(left.idempotency_key, right.idempotency_key);
                }
                _ => prop_assert!(false, "valid generated snapshot inputs were refused"),
            }
        }
        #[test]
        fn prop_fingerprint_changes_when_any_snapshot_input_changes(
            artifact in digest_strategy(),
            bootstrap in digest_strategy(),
            postgres_major in 14_u16..=17,
        ) {
            let altered_artifact = match artifact.get(1..) {
                Some(tail) if artifact.starts_with('0') => format!("1{tail}"),
                Some(tail) => format!("0{tail}"),
                None => return Ok(()),
            };
            let altered_bootstrap = match bootstrap.get(1..) {
                Some(tail) if bootstrap.starts_with('0') => format!("1{tail}"),
                Some(tail) => format!("0{tail}"),
                None => return Ok(()),
            };
            let next_major = postgres_major + 1;
            let current = preview_cluster_plan(42, &artifact, postgres_major, &bootstrap);
            let altered = preview_cluster_plan(42, &altered_artifact, postgres_major, &bootstrap);
            let altered_bootstrap = preview_cluster_plan(42, &artifact, postgres_major, &altered_bootstrap);
            let next = preview_cluster_plan(42, &artifact, next_major, &bootstrap);
            match (current, altered, altered_bootstrap, next) {
                (Some(current), Some(altered), Some(altered_bootstrap), Some(next)) => {
                    prop_assert_ne!(&current.fingerprint, &altered.fingerprint);
                    prop_assert_ne!(&current.fingerprint, &altered_bootstrap.fingerprint);
                    prop_assert_ne!(&current.fingerprint, &next.fingerprint);
                }
                _ => prop_assert!(false, "valid generated snapshot inputs were refused"),
            }
        }
        #[test]
        fn prop_noncanonical_digests_or_outside_supported_postgres_are_refused(
            artifact in ".{0,80}",
            bootstrap in ".{0,80}",
            postgres_major in any::<u16>(),
        ) {
            let canonical = |value: &str| value.len() == 64
                && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
            let plan = preview_cluster_plan(1, &artifact, postgres_major, &bootstrap);
            if !(canonical(&artifact) && canonical(&bootstrap) && (14..=18).contains(&postgres_major)) {
                prop_assert_eq!(plan, None);
            }
        }
        #[test]
        fn prop_invalid_preview_number_or_postgres_major_is_refused(
            number in any::<i32>(),
            postgres_major in any::<u16>(),
        ) {
            let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
            let plan = preview_cluster_plan(number, digest, postgres_major, digest);
            if !(1..=999_999_999).contains(&number) || !(14..=18).contains(&postgres_major) {
                prop_assert_eq!(plan, None);
            }
        }
        #[test]
        fn model_cluster_lease_reconciliation_matches_dependency_table(
            phase in prop_oneof![Just(ClusterLeasePhase::Provisioning), Just(ClusterLeasePhase::Ready), Just(ClusterLeasePhase::Deleting)],
            snapshots in prop::collection::vec(any::<bool>(), 1..32),
            clusters in prop::collection::vec(any::<bool>(), 1..32),
            connectivity in prop::collection::vec(any::<bool>(), 1..32),
        ) {
            let lease = lease(42, phase);
            for ((snapshot, cluster), connectivity) in snapshots.into_iter().zip(clusters).zip(connectivity) {
                let inventory = ClusterLeaseInventory { snapshot, cluster, connectivity };
                let action = reconcile_cluster_lease(&lease, inventory);
                prop_assert_eq!(action_name(&action), model_action(phase, inventory));
            }
        }
    }

    #[test]
    // Spec example: an uninterrupted create must establish dependencies before publishing ready.
    fn controller_walks_a_successful_lease_in_dependency_order() {
        let mut lease = lease(42, ClusterLeasePhase::Provisioning);
        let mut inventory = ClusterLeaseInventory::default();
        assert!(matches!(reconcile_cluster_lease(&lease, inventory), ClusterLeaseAction::EnsureSnapshot { .. }));
        inventory.snapshot = true;
        assert!(matches!(reconcile_cluster_lease(&lease, inventory), ClusterLeaseAction::EnsureCluster { .. }));
        inventory.cluster = true;
        assert!(matches!(reconcile_cluster_lease(&lease, inventory), ClusterLeaseAction::EnsureConnectivity { .. }));
        inventory.connectivity = true;
        assert_eq!(reconcile_cluster_lease(&lease, inventory), ClusterLeaseAction::MarkReady);
        lease.phase = ClusterLeasePhase::Ready;
        assert_eq!(reconcile_cluster_lease(&lease, inventory), ClusterLeaseAction::Noop);
    }

    #[test]
    // Spec example: closing a PR must not delete the shared snapshot.
    fn controller_never_deletes_a_shared_snapshot_with_a_pr_lease() {
        let mut lease = lease(42, ClusterLeasePhase::Provisioning);
        lease.request_deletion();
        let inventory = ClusterLeaseInventory { snapshot: true, cluster: true, connectivity: true };
        assert!(matches!(reconcile_cluster_lease(&lease, inventory), ClusterLeaseAction::DeleteConnectivity { .. }));
        let inventory = ClusterLeaseInventory { snapshot: true, cluster: true, connectivity: false };
        assert!(matches!(reconcile_cluster_lease(&lease, inventory), ClusterLeaseAction::DeleteCluster { .. }));
        let inventory = ClusterLeaseInventory { snapshot: true, cluster: false, connectivity: false };
        assert_eq!(reconcile_cluster_lease(&lease, inventory), ClusterLeaseAction::DeleteLease);
    }

    #[test]
    // Spec example: a changed fingerprint is staged while the ready preview remains attached.
    fn controller_keeps_the_ready_lease_while_a_changed_fingerprint_is_a_candidate() {
        let ready = lease(42, ClusterLeasePhase::Ready);
        let mut changed = ready.plan.clone();
        changed.fingerprint = "e".repeat(64);
        changed.cluster = "thrust-preview-pr-42-eeeeeeeeeeee".to_owned();
        changed.idempotency_key = format!("preview/42/{}", "e".repeat(64));
        let candidate = ready.replacement_candidate(changed.clone());
        assert_eq!(ready.phase, ClusterLeasePhase::Ready);
        assert!(matches!(candidate, Some(PreviewClusterLease { phase: ClusterLeasePhase::Provisioning, .. })));

        let deleting = lease(42, ClusterLeasePhase::Deleting);
        assert_eq!(deleting.replacement_candidate(changed), None);
    }

    #[test]
    fn model_cluster_lease_inventory_matrix_reaches_every_action_class() {
        let phases = [ClusterLeasePhase::Provisioning, ClusterLeasePhase::Ready, ClusterLeasePhase::Deleting];
        let values = [false, true];
        let mut seen = Vec::new();
        for phase in phases {
            for snapshot in values {
                for cluster in values {
                    for connectivity in values {
                        let action = reconcile_cluster_lease(
                            &lease(42, phase),
                            ClusterLeaseInventory { snapshot, cluster, connectivity },
                        );
                        let name = action_name(&action);
                        if !seen.contains(&name) {
                            seen.push(name);
                        }
                    }
                }
            }
        }
        assert_eq!(seen.len(), 11, "the model input matrix must reach every lease action and refusal class");
    }
    #[test]
    fn fuzz_replay_request_seeds() {
        for bytes in [
            include_bytes!("../fuzz/seeds/request/valid-json").as_slice(),
            include_bytes!("../fuzz/seeds/request/valid-path").as_slice(),
            b"".as_slice(),
        ] {
            let _ = password(bytes);
            if let Ok(path) = std::str::from_utf8(bytes) {
                let _ = route("PUT", path);
            }
        }
    }
}
