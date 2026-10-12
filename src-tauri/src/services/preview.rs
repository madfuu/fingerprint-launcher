//! What the editor shows before saving (design §3.2 / §5): the command line of a draft, the
//! arguments its kernel accepts, and the set fields that kernel would ignore.

use std::path::Path;

use rusqlite::Connection;

use crate::db::kernels::{self, Kernel};
use crate::db::profiles::ProfileDraft;
use crate::db::proxies;
use crate::error::AppResult;
use crate::kernel::args::{
    ArgSupport, FieldWarning, LaunchPlan, build_launch_plan, effective_major,
    validate_profile_for_kernel,
};
use crate::kernel::shell::HostPlatform;
use crate::services::launcher::user_data_root;

/// Program shown when no kernel is configured yet.
pub const PLACEHOLDER_PROGRAM: &str = "chrome";
/// Directory name shown for a profile that has no id yet (its real one is `<root>/<new id>`).
pub const PLACEHOLDER_PROFILE_DIR: &str = "new-profile";

/// The command line `draft` would launch with: its kernel (or the default kernel; with no
/// kernel at all, [`PLACEHOLDER_PROGRAM`] and the newest argument set), its proxy, and its
/// user data directory — its own setting, else `<root>/<profile_id>`, else
/// `<root>/`[`PLACEHOLDER_PROFILE_DIR`] for an unsaved profile.
pub fn preview_command(
    conn: &Connection,
    default_root: &Path,
    draft: &ProfileDraft,
    profile_id: Option<&str>,
) -> AppResult<LaunchPlan> {
    let kernel = kernel_for(conn, draft.kernel_id.as_deref())?.unwrap_or_else(placeholder_kernel);
    let proxy = draft
        .proxy_id
        .as_deref()
        .map(|id| proxies::get(conn, id))
        .transpose()?;
    let user_data_dir = match non_blank(draft.user_data_dir.as_deref()) {
        Some(dir) => Path::new(dir).to_owned(),
        None => user_data_root(conn, default_root)?
            .join(non_blank(profile_id).unwrap_or(PLACEHOLDER_PROFILE_DIR)),
    };
    build_launch_plan(
        draft,
        &kernel,
        proxy.as_ref(),
        &user_data_dir,
        HostPlatform::current(),
    )
}

/// The argument set of `kernel_id`, or of the default kernel when `None`; the newest set when
/// no kernel is configured (design §3.2).
pub fn arg_support(conn: &Connection, kernel_id: Option<&str>) -> AppResult<ArgSupport> {
    let kernel = kernel_for(conn, kernel_id)?;
    Ok(ArgSupport::for_major(effective_major(kernel.as_ref())))
}

/// Set fields of `draft` that its kernel (see [`arg_support`]) would not pass on.
pub fn validate_for_kernel(
    conn: &Connection,
    draft: &ProfileDraft,
) -> AppResult<Vec<FieldWarning>> {
    let kernel = kernel_for(conn, draft.kernel_id.as_deref())?;
    Ok(validate_profile_for_kernel(
        draft,
        effective_major(kernel.as_ref()),
    ))
}

/// The kernel with this id (`not_found` if it is gone), or the default kernel.
fn kernel_for(conn: &Connection, kernel_id: Option<&str>) -> AppResult<Option<Kernel>> {
    match non_blank(kernel_id) {
        Some(id) => kernels::get(conn, id).map(Some),
        None => kernels::get_default(conn),
    }
}

fn placeholder_kernel() -> Kernel {
    Kernel {
        id: String::new(),
        name: String::new(),
        exec_path: PLACEHOLDER_PROGRAM.to_owned(),
        major_version: effective_major(None),
        full_version: None,
        is_default: false,
        created_at: String::new(),
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::TestDb;
    use crate::error::ErrorCode;
    use crate::kernel::args::GatedField;

    fn draft() -> ProfileDraft {
        ProfileDraft {
            fingerprint_seed: Some(1000),
            fp_gpu_vendor: Some("NVIDIA".into()),
            disable_spoofing: vec![crate::db::profiles::SpoofingFeature::Gpu],
            ..TestDb::draft("p")
        }
    }

    #[test]
    fn without_any_kernel_uses_a_placeholder_and_the_newest_arguments() {
        let db = TestDb::new();
        let root = Path::new(&db.abs_path("root")).to_owned();
        let plan = preview_command(&db.conn, &root, &draft(), None).unwrap();
        assert_eq!(plan.program, Path::new(PLACEHOLDER_PROGRAM));
        assert_eq!(
            plan.args,
            [
                format!(
                    "--user-data-dir={}",
                    root.join(PLACEHOLDER_PROFILE_DIR).display()
                ),
                "--fingerprint=1000".to_owned(),
                "--disable-spoofing=gpu".to_owned(),
                "--disable-non-proxied-udp".to_owned(),
            ]
        );
        assert!(plan.shell_line.starts_with("chrome "));

        assert_eq!(
            arg_support(&db.conn, None).unwrap(),
            ArgSupport::for_major(u32::MAX)
        );
        let warnings = validate_for_kernel(&db.conn, &draft()).unwrap();
        assert_eq!(
            warnings.iter().map(|w| w.field).collect::<Vec<_>>(),
            [GatedField::FpGpuVendor]
        );
    }

    #[test]
    fn uses_the_bound_or_default_kernel_and_the_profile_directory() {
        let db = TestDb::new();
        let root = Path::new(&db.abs_path("root")).to_owned();
        let k142 = db.kernel("k142", 142); // first kernel: the default
        let k150 = db.kernel("k150", 150);

        let plan = preview_command(&db.conn, &root, &draft(), Some("abc")).unwrap();
        assert_eq!(plan.program, Path::new(&k142.exec_path));
        assert_eq!(
            plan.args[..3],
            [
                format!("--user-data-dir={}", root.join("abc").display()),
                "--fingerprint=1000".to_owned(),
                "--fingerprint-gpu-vendor=NVIDIA".to_owned(),
            ]
        );

        let bound = ProfileDraft {
            kernel_id: Some(k150.id.clone()),
            user_data_dir: Some(db.abs_path("own")),
            ..draft()
        };
        let plan = preview_command(&db.conn, &root, &bound, Some("abc")).unwrap();
        assert_eq!(plan.program, Path::new(&k150.exec_path));
        assert_eq!(
            plan.args[0],
            format!("--user-data-dir={}", db.abs_path("own"))
        );
        assert!(plan.args.contains(&"--disable-spoofing=gpu".to_owned()));

        assert!(
            arg_support(&db.conn, Some(&k142.id))
                .unwrap()
                .gpu_vendor_renderer
        );
        assert!(
            arg_support(&db.conn, Some(&k150.id))
                .unwrap()
                .disable_spoofing
        );
        let fields = |d: &ProfileDraft| {
            validate_for_kernel(&db.conn, d)
                .unwrap()
                .iter()
                .map(|w| w.field)
                .collect::<Vec<_>>()
        };
        assert_eq!(fields(&draft()), [GatedField::DisableSpoofing]);
        assert_eq!(fields(&bound), [GatedField::FpGpuVendor]);
    }

    #[test]
    fn dangling_references_are_not_found() {
        let db = TestDb::new();
        let root = Path::new(&db.abs_path("root")).to_owned();
        for draft in [
            ProfileDraft {
                kernel_id: Some("gone".into()),
                ..draft()
            },
            ProfileDraft {
                proxy_id: Some("gone".into()),
                ..draft()
            },
        ] {
            let err = preview_command(&db.conn, &root, &draft, None).unwrap_err();
            assert_eq!(err.code, ErrorCode::NotFound);
        }
        let err = arg_support(&db.conn, Some("gone")).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }
}
