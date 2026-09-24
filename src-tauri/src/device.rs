use serde::Serialize;
use std::path::Path;
use std::process::Command;

use crate::util;

// Device module, scoped by risk:
//   - read-only: adb/fastboot detection and info gathering
//   - reversible: reboots into bootloader / fastbootd / recovery
//   - guarded: fastboot flash, with identity partitions refused outright,
//     the bootloader chain behind an explicit opt-in, image-vs-partition
//     size checks, and a fastbootd requirement for dynamic partitions.
// No Samsung download mode / Odin protocol here yet - that is a separate
// protocol and is not faked.

fn log(app: &tauri::AppHandle, line: String) {
    use tauri::Emitter;
    let _ = app.emit("log-event", line);
}

// ---- parsing helpers (unit-tested) ----

/// `adb devices` output -> (serial, state) pairs.
pub fn parse_adb_devices(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .skip(1)
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                return None;
            }
            let mut it = l.split_whitespace();
            match (it.next(), it.next()) {
                (Some(s), Some(st)) => Some((s.to_string(), st.to_string())),
                _ => None,
            }
        })
        .collect()
}

/// `fastboot getvar all` output -> variable map. Lines look like
/// "(bootloader) current-slot: b" and the tool writes them to stderr.
pub fn parse_fastboot_vars(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("(bootloader)").unwrap_or(line).trim();
        if let Some((k, v)) = line.rsplit_once(':') {
            let k = k.trim();
            let v = v.trim();
            if !k.is_empty() && !k.contains(' ') {
                out.push((k.to_string(), v.to_string()));
            }
        }
    }
    out
}

/// fastboot partition sizes are hex strings like "0x20000000".
pub fn parse_hex_size(v: &str) -> Option<u64> {
    let v = v.trim().trim_start_matches("0x").trim_start_matches("0X");
    u64::from_str_radix(v, 16).ok()
}

/// Partitions that hold device identity, calibration or radio state: a bad
/// write here is unrecoverable even with the right image, so this tool
/// refuses them outright on every transport.
pub fn is_identity_partition(partition: &str) -> bool {
    let p = partition
        .trim_end_matches("_a")
        .trim_end_matches("_b")
        .to_lowercase();
    matches!(
        p.as_str(),
        "modemst1" | "modemst2" | "modemst" | "fsc" | "fsg" | "persist" | "persistbak" | "efs"
            | "nvdata" | "nvram" | "secdata" | "oplusstanvbk" | "oplussec" | "mi_ext" | "cust"
    )
}

/// The bootloader chain: flashable by experts, but this tool only proceeds
/// with an explicit opt-in and names the partition in the confirmation.
pub fn is_bootloader_partition(partition: &str) -> bool {
    let p = partition
        .trim_end_matches("_a")
        .trim_end_matches("_b")
        .to_lowercase();
    matches!(
        p.as_str(),
        "bootloader" | "xbl" | "xbl_config" | "abl" | "aop" | "aop_config" | "tz" | "hyp"
            | "cmnlib" | "cmnlib64" | "keymaster" | "km" | "uefisecapp" | "devcfg" | "qupfw"
            | "vmboo" | "shrm" | "imagefv" | "multiimgoem" | "featenabler" | "cpucp"
    )
}

/// Dynamic partitions live inside super: writing them needs fastbootd
/// (userspace fastboot), not bootloader fastboot.
pub fn is_dynamic_partition(partition: &str) -> bool {
    let p = partition
        .trim_end_matches("_a")
        .trim_end_matches("_b")
        .to_lowercase();
    matches!(p.as_str(), "system" | "vendor" | "product" | "odm" | "system_ext" | "vendor_dlkm" | "odm_dlkm" | "system_dlkm")
}

// ---- read-only commands ----

#[derive(Debug, Serialize)]
pub struct DeviceEntry {
    pub serial: String,
    pub state: String,
}

fn run_capture(program: &str, args: &[&str]) -> Result<String, String> {
    if util::find_tool(program).is_none() {
        return Err(format!(
            "{} not found on PATH (pacman: android-tools)",
            program
        ));
    }
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run {}: {}", program, e))?;
    // fastboot getvar writes to stderr; adb writes to stdout. Combine both.
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(text)
}

#[tauri::command]
pub async fn adb_devices() -> Result<Vec<DeviceEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let text = run_capture("adb", &["devices"])?;
        Ok(parse_adb_devices(&text)
            .into_iter()
            .map(|(serial, state)| DeviceEntry { serial, state })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn fastboot_devices() -> Result<Vec<DeviceEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let text = run_capture("fastboot", &["devices"])?;
        // "SERIAL\tfastboot"
        Ok(text
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                match (it.next(), it.next()) {
                    (Some(s), Some(st)) if st.contains("fastboot") => {
                        Some(DeviceEntry { serial: s.to_string(), state: "fastboot".into() })
                    }
                    _ => None,
                }
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn adb_device_info(serial: String) -> Result<Vec<(String, String)>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let text = run_capture("adb", &["-s", &serial, "shell", "getprop"])?;
        let mut out = Vec::new();
        for line in text.lines() {
            // "[ro.product.model]: [SM-X115]"
            let line = line.trim();
            if !(line.starts_with('[') && line.ends_with(']')) {
                continue;
            }
            let inner = &line[1..line.len() - 1];
            if let Some((k, v)) = inner.split_once("]: [") {
                out.push((k.to_string(), v.to_string()));
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn fastboot_device_vars() -> Result<Vec<(String, String)>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let text = run_capture("fastboot", &["getvar", "all"])?;
        Ok(parse_fastboot_vars(&text))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- reboots (reversible) ----

#[tauri::command]
pub async fn adb_reboot(serial: String, target: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let allowed = ["bootloader", "recovery", "fastbootd", "download", "sideload"];
        if !allowed.contains(&target.as_str()) {
            return Err(format!("unsupported reboot target: {}", target));
        }
        let arg = if target == "fastbootd" { "fastboot".to_string() } else { target.clone() };
        run_capture("adb", &["-s", &serial, "reboot", &arg])?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn fastboot_reboot(serial: String, target: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let allowed = ["bootloader", "fastbootd", "recovery"];
        if !allowed.contains(&target.as_str()) {
            return Err(format!("unsupported reboot target: {}", target));
        }
        let arg = if target == "fastbootd" { "reboot-fastboot".to_string() } else { format!("reboot-{}", target) };
        run_capture("fastboot", &["-s", &serial, &arg])?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- guarded flashing ----

#[derive(Debug, Serialize)]
pub struct FlashPlan {
    pub partition: String,
    pub image: String,
    pub image_size: u64,
    pub partition_size: Option<u64>,
    pub dynamic: bool,
    pub userspace_fastboot: Option<bool>,
    pub identity: bool,
    pub bootloader: bool,
    /// Facts worth reading before proceeding.
    pub warnings: Vec<String>,
    /// Operations that cannot physically succeed right now - the flash
    /// button stays disabled.
    pub blockers: Vec<String>,
    /// Risk acknowledgements the user must check before the button appears.
    pub unacknowledged: Vec<String>,
}

/// Build the flash decision without touching the device: every check is
/// visible to the user before anything is written.
///
/// Policy: the tool informs and requires explicit acknowledgement, it does
/// not decide for the user - this is a power-user tool, and hard blocks
/// would be inconsistent with the custom-script and kernel-patching
/// capabilities it already has. The one exception is an operation that
/// cannot succeed at all (image larger than the partition): fastboot would
/// fail mid-write, which is the worst possible failure mode, so the plan
/// blocks it up front.
fn plan_flash(
    partition: &str,
    image: &str,
    vars: &[(String, String)],
    bootloader_opt_in: bool,
    identity_opt_in: bool,
    dynamic_ack: bool,
) -> Result<FlashPlan, String> {
    let image_path = Path::new(image);
    if !image_path.is_file() {
        return Err(format!("image not found: {}", image));
    }
    let image_size = std::fs::metadata(image_path).map_err(|e| e.to_string())?.len();
    let dynamic = is_dynamic_partition(partition);
    let identity = is_identity_partition(partition);
    let bootloader = is_bootloader_partition(partition);
    let userspace_fastboot = vars
        .iter()
        .find(|(k, _)| k == "is-userspace")
        .map(|(_, v)| v == "yes");
    let partition_size = vars
        .iter()
        .find(|(k, _)| k == &format!("partition-size:{}", partition))
        .and_then(|(_, v)| parse_hex_size(v));

    let mut warnings = Vec::new();
    let mut blockers = Vec::new();
    let mut unacknowledged = Vec::new();

    if let Some(psize) = partition_size {
        if image_size > psize {
            blockers.push(format!(
                "image ({} bytes) is larger than partition {} ({} bytes) - fastboot would fail mid-write",
                image_size,
                partition,
                psize
            ));
        }
    }

    if identity {
        if identity_opt_in {
            warnings.push(format!(
                "IDENTITY PARTITION OVERRIDE: {} holds IMEI/radio/calibration state. Without a backup of this exact partition, overwriting it may permanently lose it.",
                partition
            ));
        } else {
            unacknowledged.push(format!(
                "{} holds device identity/radio state (IMEI, calibration). Overwriting it without a backup cannot be undone - nothing can put it back. Proceed only with your own backup of this partition.",
                partition
            ));
        }
    }
    if bootloader {
        if bootloader_opt_in {
            warnings.push(format!(
                "BOOTLOADER CHAIN OVERRIDE: a bad write to {} can permanently brick the device.",
                partition
            ));
        } else {
            unacknowledged.push(format!(
                "{} is part of the bootloader chain - a bad write here can permanently brick the device.",
                partition
            ));
        }
    }
    if dynamic && userspace_fastboot != Some(true) {
        let note = format!(
            "{} is a dynamic partition and the device is in bootloader fastboot - most devices need fastbootd for this; fastboot will error immediately if this one does not support it.",
            partition
        );
        if dynamic_ack {
            warnings.push(note);
        } else {
            unacknowledged.push(note);
        }
    }

    Ok(FlashPlan {
        partition: partition.to_string(),
        image: image.to_string(),
        image_size,
        partition_size,
        dynamic,
        userspace_fastboot,
        identity,
        bootloader,
        warnings,
        blockers,
        unacknowledged,
    })
}

#[tauri::command]
pub async fn flash_plan(
    partition: String,
    image: String,
    bootloader_opt_in: bool,
    identity_opt_in: bool,
    dynamic_ack: bool,
) -> Result<FlashPlan, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let text = run_capture("fastboot", &["getvar", "all"])?;
        let vars = parse_fastboot_vars(&text);
        plan_flash(
            &partition,
            &image,
            &vars,
            bootloader_opt_in,
            identity_opt_in,
            dynamic_ack,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn flash_image(
    app_handle: tauri::AppHandle,
    partition: String,
    image: String,
    bootloader_opt_in: bool,
    identity_opt_in: bool,
    dynamic_ack: bool,
) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let text = run_capture("fastboot", &["getvar", "all"])?;
        let vars = parse_fastboot_vars(&text);
        let plan = plan_flash(&partition, &image, &vars, bootloader_opt_in, identity_opt_in, dynamic_ack)?;
        if !plan.blockers.is_empty() {
            return Err(format!("flash impossible: {}", plan.blockers.join("; ")));
        }
        if !plan.unacknowledged.is_empty() {
            return Err(format!(
                "unacknowledged risks (confirm them in the UI): {}",
                plan.unacknowledged.join("; ")
            ));
        }
        for w in &plan.warnings {
            log(&app, format!("[Flash] {}", w));
        }
        log(&app, format!(
            "[Flash] writing {} ({} bytes) to {}{}",
            image,
            plan.image_size,
            partition,
            if plan.dynamic { " (fastbootd)" } else { "" }
        ));
        let mut cmd = Command::new("fastboot");
        cmd.arg("flash").arg(&partition).arg(&image);
        util::run_logged(&app, "fastboot", cmd)?;
        log(&app, format!("[Flash] {} written - verify before rebooting if this was your only copy", partition));
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_adb_devices() {
        let out = "List of devices attached\nSERIAL1\tdevice\nSERIAL2\toffline\n\n";
        let got = parse_adb_devices(out);
        assert_eq!(got, vec![("SERIAL1".into(), "device".into()), ("SERIAL2".into(), "offline".into())]);
        assert!(parse_adb_devices("").is_empty());
    }

    #[test]
    fn parses_fastboot_vars() {
        let out = "(bootloader) current-slot: b\n(bootloader) partition-size:system_a: 0x20000000\nOKAY";
        let vars = parse_fastboot_vars(out);
        assert!(vars.contains(&("current-slot".into(), "b".into())));
        assert!(vars.contains(&("partition-size:system_a".into(), "0x20000000".into())));
        assert_eq!(parse_hex_size("0x20000000"), Some(0x20000000));
        assert_eq!(parse_hex_size("garbage"), None);
    }

    #[test]
    fn identity_partitions_are_refused_regardless_of_suffix() {
        for p in ["modemst1", "modemst2_a", "persist", "persist_b", "fsc", "nvdata"] {
            assert!(is_identity_partition(p), "{} should be identity", p);
        }
        // Slot suffixes strip, but names must not false-positive.
        assert!(!is_identity_partition("system"));
        assert!(!is_identity_partition("boot_a"));
    }

    #[test]
    fn risky_partitions_need_acknowledgement_not_refusal() {
        assert!(is_bootloader_partition("xbl_a"));
        assert!(is_bootloader_partition("keymaster"));
        assert!(!is_bootloader_partition("boot"));
        assert!(is_dynamic_partition("system_a"));
        assert!(is_dynamic_partition("vendor"));
        assert!(!is_dynamic_partition("boot_b"));

        // plan_flash requires a real image file; make one that fits 0x1000.
        let tmp = std::env::temp_dir().join(format!("tk-flash-fit-{}", std::process::id()));
        std::fs::write(&tmp, vec![0u8; 16]).unwrap();
        let img = tmp.to_string_lossy().into_owned();

        // Clean case: nothing risky, nothing asked.
        let vars = vec![
            ("is-userspace".to_string(), "yes".to_string()),
            ("partition-size:system_a".to_string(), format!("{:#x}", 4096)),
        ];
        let plan = plan_flash("system_a", &img, &vars, false, false, false).unwrap();
        assert!(plan.blockers.is_empty() && plan.unacknowledged.is_empty() && plan.warnings.is_empty());

        // Dynamic partition from bootloader fastboot: acknowledged warning,
        // never a refusal - some bootloaders do support it.
        let vars_bl = vec![("is-userspace".to_string(), "no".to_string())];
        let plan = plan_flash("system_a", &img, &vars_bl, false, false, false).unwrap();
        assert!(plan.blockers.is_empty());
        assert!(plan.unacknowledged.iter().any(|m| m.contains("fastbootd")));
        let plan = plan_flash("system_a", &img, &vars_bl, false, false, true).unwrap();
        assert!(plan.unacknowledged.is_empty());
        assert!(plan.warnings.iter().any(|m| m.contains("fastbootd")));

        // Bootloader chain: unacknowledged until opted in.
        let plan = plan_flash("xbl", &img, &vars_bl, false, false, true).unwrap();
        assert!(plan.unacknowledged.iter().any(|m| m.contains("bootloader chain")));
        let plan = plan_flash("xbl", &img, &vars_bl, true, false, true).unwrap();
        assert!(plan.unacknowledged.is_empty());
        assert!(plan.warnings.iter().any(|m| m.contains("BOOTLOADER CHAIN OVERRIDE")));

        // Identity partitions: same model, stronger wording.
        let vars_persist = vec![];
        let plan = plan_flash("persist", &img, &vars_persist, false, false, false).unwrap();
        assert!(plan.identity);
        assert!(plan.unacknowledged.iter().any(|m| m.contains("cannot be undone")));
        let plan = plan_flash("persist", &img, &vars_persist, false, true, false).unwrap();
        assert!(plan.unacknowledged.is_empty());
        assert!(plan.warnings.iter().any(|m| m.contains("IDENTITY PARTITION OVERRIDE")));

        std::fs::remove_file(&tmp).unwrap();
    }

    #[test]
    fn oversized_images_block_even_with_every_opt_in() {
        let vars = vec![
            ("is-userspace".to_string(), "yes".to_string()),
            ("partition-size:boot_a".to_string(), "0x400".to_string()),
        ];
        let tmp = std::env::temp_dir().join(format!("tk-flash-{}", std::process::id()));
        std::fs::write(&tmp, vec![0u8; 1025]).unwrap();
        let plan = plan_flash("boot_a", &tmp.to_string_lossy(), &vars, true, true, true).unwrap();
        assert!(plan.blockers.iter().any(|m| m.contains("larger than partition")));
        std::fs::remove_file(&tmp).unwrap();
    }
}
