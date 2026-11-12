//! Assets (issue #44, the asset stage; ADR-0047): a file the stock app uploads, which for now is a project's cover.
//!
//! The lifecycle is ADR-0047's: `create_asset` makes a pending asset and a one-time grant to upload its bytes (the browser sends
//! the upload with no cookies, so the grant is a token in the address it is given); [`receive_upload`] spends the grant and
//! stores the bytes; [`confirm_upload`] looks at the object and only then marks the asset uploaded, where the pin trusts the
//! client (D-035); [`attach_project_cover`] gives an uploaded cover to the project it was made for; [`open_asset`] and
//! [`public_asset`] say who may read one.

use thiserror::Error;
use thrust_contracts::*;
use thrust_ports::*;

use crate::{authorize, hash_token, random_hex, App, CoreError, Permission, Principal};

/// The image types an upload may have: the pin's list for S28, which is also the list it shows inline.
pub const IMAGE_TYPES: [&str; 5] = ["image/jpeg", "image/png", "image/webp", "image/jpg", "image/gif"];

/// The longest file name kept, in bytes. The pin lets a name run to the length of its storage key (800) and answers a longer one
/// with a 500; this is a 400, and a key is never near a store's limit.
pub const MAX_ASSET_NAME: usize = 255;

/// Why an asset's routes refuse a request, in the cases the pin tells apart (and the ones Thrust adds).
#[derive(Debug, Error)]
pub enum AssetError {
    /// No live asset of the workspace has that id.
    #[error("no such asset")]
    NotFound,
    /// The asset belongs to a project the caller is not a member of (the pin's `has_project_asset_access`).
    #[error("no access to this asset")]
    NoAccess,
    /// The asset has no bytes yet: made but not uploaded and confirmed.
    #[error("the asset is not uploaded")]
    NotUploaded,
    /// A file type outside [`IMAGE_TYPES`].
    #[error("the file type is not allowed")]
    BadType,
    /// A size under one byte.
    #[error("the size is not a positive number of bytes")]
    BadSize,
    /// A name that is longer than [`MAX_ASSET_NAME`] once it is sanitised.
    #[error("the file name is too long")]
    BadName,
    /// An upload with a token that names no pending asset, or that was spent or has expired. The three look alike on purpose.
    #[error("the upload address is not valid")]
    NoGrant,
    /// An upload whose body or type is not what the asset was made for.
    #[error("the upload is not what was asked for")]
    UploadMismatch,
    /// Confirming an asset whose object is not in the store.
    #[error("no object has been uploaded for the asset")]
    ObjectMissing,
    /// Confirming an asset whose object has another size or type than the asset says.
    #[error("the uploaded object is not what the asset says")]
    ObjectMismatch,
    /// A bulk attach that names no asset.
    #[error("no asset ids")]
    NoIds,
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<DbError> for AssetError {
    fn from(e: DbError) -> Self {
        AssetError::Core(e.into())
    }
}

impl From<BlobError> for AssetError {
    fn from(e: BlobError) -> Self {
        AssetError::Core(e.into())
    }
}

/// The pin's `sanitize_filename` (`apps/api/plane/utils/path_validator.py`): what a client's file name becomes before it is a
/// part of an object's key and of a download's name. ASCII control characters are dropped, a backslash counts as a slash, only
/// what follows the last slash is kept, every `..` is removed, and leading dots and the whitespace around it are stripped.
/// `None` when nothing is left.
pub fn sanitize_filename(name: &str) -> Option<String> {
    let without_controls: String = name.chars().filter(|c| !(u32::from(*c) < 32 || *c == '\u{7f}')).collect();
    let slashed = without_controls.replace('\\', "/");
    // `os.path.basename`: what follows the last slash, which is nothing when the name ends in one
    let base = slashed.rsplit('/').next().unwrap_or("");
    let no_traversal = base.replace("..", "");
    let cleaned = no_traversal.trim().trim_start_matches('.').trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

/// What S28 asks for. `name` and `size` are what the client sent, unchecked: a missing name is "unnamed", a missing size is the
/// instance's limit, and a size above it is held to it (the pin clamps in silence).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUpload {
    pub name: Option<String>,
    pub content_type: String,
    pub size: Option<i64>,
    pub entity: AssetEntity,
}

/// A made asset and the token its upload address carries. The token is shown once, here; the row keeps only its digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadTicket {
    pub asset: AssetRow,
    pub token: String,
}

fn same_type(a: &str, b: &str) -> bool {
    // a media type is case-insensitive, and an upload may add parameters (`; charset=...`) that are not part of the type
    let base = |s: &str| s.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    base(a) == base(b)
}

/// S28. Make a pending asset of the scope's workspace for the caller, with a grant to upload its bytes.
pub async fn create_asset(app: &App, scope: &WorkspaceScope, req: NewUpload) -> Result<UploadTicket, AssetError> {
    if !IMAGE_TYPES.contains(&req.content_type.as_str()) {
        return Err(AssetError::BadType);
    }
    let limit = i64::try_from(app.cfg.max_attachment_bytes).unwrap_or(i64::MAX);
    let size = req.size.map_or(limit, |s| s.min(limit));
    if size < 1 {
        return Err(AssetError::BadSize);
    }
    let name = req.name.as_deref().and_then(sanitize_filename).unwrap_or_else(|| "unnamed".to_string());
    if name.len() > MAX_ASSET_NAME {
        return Err(AssetError::BadName);
    }
    let id = AssetId(app.ports.ids.new_uuid());
    let now = app.ports.clock.now();
    // the pin's key is the workspace's, a random part and the name; this one's random part is the asset's own id
    let object_key = format!("{}/{}-{}", scope.workspace_id(), id.0.simple(), name);
    let token = random_hex(app, 32)?;
    let asset = AssetRow {
        id,
        workspace_id: scope.workspace_id(),
        created_by: scope.user_id(),
        entity: req.entity,
        project_id: None,
        name,
        content_type: req.content_type,
        size,
        object_key,
        state: AssetState::Pending,
        upload: Some(UploadGrant {
            token_hash: hash_token(&token),
            expires_at: now + thrust_contracts::chrono::TimeDelta::seconds(app.cfg.asset_upload_ttl_secs),
        }),
        created_at: now,
        uploaded_at: None,
        deleted_at: None,
    };
    app.ports.db.insert_asset(scope, &asset).await?;
    Ok(UploadTicket { asset, token })
}

/// The upload itself, made by the browser with no session: spend the grant `token` names, check that the body is the file the asset
/// was made for (its type, and exactly its size, as a signed PUT pins both), and store it. A grant is spent by the first upload that
/// holds it, even one that is refused here, so a refused upload is made again from S28.
pub async fn receive_upload(
    app: &App,
    id: AssetId,
    token: &str,
    content_type: Option<&str>,
    bytes: Vec<u8>,
) -> Result<(), AssetError> {
    let asset = app
        .ports
        .db
        .take_upload_grant(id, &hash_token(token), app.ports.clock.now())
        .await?
        .ok_or(AssetError::NoGrant)?;
    let sent_the_type = content_type.is_some_and(|t| same_type(t, &asset.content_type));
    let sent_the_size = i64::try_from(bytes.len()).is_ok_and(|n| n == asset.size);
    if !sent_the_type || !sent_the_size {
        return Err(AssetError::UploadMismatch);
    }
    app.ports.blobs.put(&asset.object_key, bytes, Some(asset.content_type)).await?;
    Ok(())
}

/// Whether the caller may reach an asset through a route of the workspace: one that belongs to no project is the workspace's, and
/// one that belongs to a project needs the caller to be in that project (the pin's `has_project_asset_access`).
async fn may_reach(app: &App, scope: &WorkspaceScope, asset: &AssetRow) -> Result<bool, AssetError> {
    match asset.project_id {
        None => Ok(true),
        Some(project) => Ok(app.ports.db.project_member(scope, project).await?.is_some()),
    }
}

/// S29. The caller says the upload is done. The pin marks the asset uploaded without looking; this asks the store first (ADR-0047's
/// `confirmUpload`): the object must be there with the size and type the asset says. An asset that is uploaded already stays so.
pub async fn confirm_upload(app: &App, scope: &WorkspaceScope, id: AssetId) -> Result<AssetRow, AssetError> {
    let mut asset = app.ports.db.asset(scope, id).await?.ok_or(AssetError::NotFound)?;
    if !may_reach(app, scope, &asset).await? {
        return Err(AssetError::NoAccess);
    }
    if asset.state == AssetState::Uploaded {
        return Ok(asset);
    }
    let head = app.ports.blobs.head(&asset.object_key).await?.ok_or(AssetError::ObjectMissing)?;
    let right_type = head.content_type.as_deref().is_some_and(|t| same_type(t, &asset.content_type));
    let right_size = i64::try_from(head.size).is_ok_and(|n| n == asset.size);
    if !right_size || !right_type {
        return Err(AssetError::ObjectMismatch);
    }
    let now = app.ports.clock.now();
    if !app.ports.db.mark_asset_uploaded(scope, id, now).await? {
        return Err(AssetError::NotFound);
    }
    asset.state = AssetState::Uploaded;
    asset.uploaded_at = Some(now);
    asset.upload = None;
    Ok(asset)
}

/// S33 and the route it sends the browser to. The asset, when the caller may read it: it is in the workspace, the caller may reach
/// it, and it has its bytes.
pub async fn open_asset(app: &App, scope: &WorkspaceScope, id: AssetId) -> Result<AssetRow, AssetError> {
    let asset = app.ports.db.asset(scope, id).await?.ok_or(AssetError::NotFound)?;
    if !may_reach(app, scope, &asset).await? {
        return Err(AssetError::NoAccess);
    }
    if asset.state != AssetState::Uploaded {
        return Err(AssetError::NotUploaded);
    }
    Ok(asset)
}

/// The address a project's cover is shown at, which needs no session (the pin's `AllowAny`): an uploaded asset of a kind that is
/// shown to anyone. A cover is the only kind there is, and its address holds a random id, as the pin's does. An asset that does not
/// exist and one that is not uploaded are not told apart: nothing is learned without a session.
pub async fn public_asset(app: &App, id: AssetId) -> Result<AssetRow, AssetError> {
    match app.ports.db.uploaded_asset(id).await? {
        Some(a) if a.entity == AssetEntity::ProjectCover => Ok(a),
        _ => Err(AssetError::NotUploaded),
    }
}

/// An asset's bytes. An uploaded asset whose object is gone is a missing asset.
pub async fn read_asset(app: &App, asset: &AssetRow) -> Result<Blob, AssetError> {
    app.ports.blobs.get(&asset.object_key).await?.ok_or(AssetError::NotFound)
}

/// S31. Attach the caller's own uploads to the project, which the caller must be a member of; the oldest of them becomes its
/// cover (see [`Database::attach_project_cover`]). Anything of `ids` that is not the caller's, or belongs to another project, is
/// left out without a word, as the pin does; none left is `NotFound`.
pub async fn attach_project_cover(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    ids: &[AssetId],
) -> Result<AssetId, AssetError> {
    let scope = authorize(app, who, slug, project, Permission::ReadProject).await?;
    if ids.is_empty() {
        return Err(AssetError::NoIds);
    }
    let now = app.ports.clock.now();
    app.ports.db.attach_project_cover(&scope, ids, now).await?.ok_or(AssetError::NotFound)
}

/// Whether `id` may be named as the cover of a project (`project` is `None` while the project is being made): an uploaded cover
/// that is the project's own already (a save that repeats the cover it has, whoever uploaded it), or that belongs to no project and
/// is the caller's. The pin accepts the id of any asset at all, another workspace's included; this is S31's rule, but for the
/// repeat, which S31 is never asked to attach again.
pub async fn is_attachable_cover(
    app: &App,
    scope: &WorkspaceScope,
    id: AssetId,
    project: Option<ProjectId>,
) -> Result<bool, CoreError> {
    Ok(app.ports.db.asset(scope, id).await?.is_some_and(|a| {
        a.entity == AssetEntity::ProjectCover
            && a.state == AssetState::Uploaded
            && match a.project_id {
                Some(p) => Some(p) == project,
                None => a.created_by == scope.user_id(),
            }
    }))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    // (b) the examples the pin's own docstring and code name: traversal, Windows paths, hidden files, control characters
    #[rstest::rstest]
    #[case("cover.png", Some("cover.png"))]
    #[case("../../etc/passwd", Some("passwd"))]
    #[case("..\\..\\windows\\win.ini", Some("win.ini"))]
    #[case("a/b/c.png", Some("c.png"))]
    #[case("dir/", None)]
    #[case(".env", Some("env"))]
    #[case(" .env", Some("env"))]
    #[case("..", None)]
    #[case("...", None)]
    #[case("a..b.png", Some("ab.png"))]
    #[case("my\u{0}file\n.png", Some("myfile.png"))]
    #[case("", None)]
    #[case("   ", None)]
    #[case("\u{7f}\u{1}", None)]
    #[case("naïve – ファイル.png", Some("naïve – ファイル.png"))]
    fn sanitize_filename_cases(#[case] input: &str, #[case] expected: Option<&str>) {
        assert_eq!(sanitize_filename(input).as_deref(), expected, "{input:?}");
    }

    proptest! {
        // what comes out is one safe path segment, whatever went in
        #[test]
        fn prop_a_sanitised_name_is_one_safe_segment(name in any::<String>()) {
            if let Some(clean) = sanitize_filename(&name) {
                prop_assert!(!clean.is_empty());
                prop_assert!(!clean.contains('/') && !clean.contains('\\'), "{clean:?}");
                prop_assert!(!clean.contains(".."), "{clean:?}");
                prop_assert!(!clean.starts_with('.'), "{clean:?}");
                prop_assert!(clean.chars().all(|c| u32::from(c) >= 32 && c != '\u{7f}'), "{clean:?}");
                prop_assert_eq!(clean.trim(), clean.as_str());
            }
        }

        // cleaning twice changes nothing more
        #[test]
        fn prop_sanitising_is_idempotent(name in any::<String>()) {
            if let Some(once) = sanitize_filename(&name) {
                prop_assert_eq!(sanitize_filename(&once), Some(once));
            }
        }

        // a name that is already one plain segment passes unchanged
        #[test]
        fn prop_a_plain_name_is_kept(name in "[A-Za-z0-9_-][A-Za-z0-9_ -]{0,40}\\.(png|jpg|gif|webp)") {
            prop_assert_eq!(sanitize_filename(&name), Some(name));
        }
    }
}
