//! Compositor-owned shortcuts. Never replace an existing user binding.
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const BEGIN: &str = "\n// BEGIN Tap autoclicker shortcut\n";
const END: &str = "// END Tap autoclicker shortcut\n";

pub enum Compositor {
    Hyprland {
        lua: bool,
        key: Option<String>,
    },
    Niri {
        config: PathBuf,
        installed: Option<String>,
    },
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("Cannot run {program}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

fn hypr(args: &[&str]) -> Result<String, String> {
    let reply = run("hyprctl", args)?;
    if reply.trim() != "ok" {
        return Err(format!(
            "Hyprland could not set the shortcut: {}",
            reply.trim()
        ));
    }
    Ok(reply)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn executable() -> Result<String, String> {
    // AppImage's current_exe points into a temporary mount. Use its stable launcher.
    let path = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(std::env::current_exe)
        .map_err(|e| e.to_string())?;
    path.into_os_string()
        .into_string()
        .map_err(|_| "The executable path must be valid UTF-8.".into())
}

fn hypr_conflict(binds: &Value, key: &str) -> Result<bool, String> {
    let binds = binds
        .as_array()
        .ok_or("Hyprland returned an invalid binding list.")?;
    // XKB keycodes, including the +8 offset from evdev, for F1 through F12.
    let number: u32 = key[1..].parse().map_err(|_| "Invalid function key")?;
    let code = if number <= 10 {
        66 + number
    } else {
        84 + number
    };
    Ok(binds.iter().any(|bind| {
        bind["modmask"].as_u64() == Some(0)
            && (bind["key"].as_str().is_some_and(|name| {
                name.split('&')
                    .any(|part| part.trim().eq_ignore_ascii_case(key))
            }) || bind["keycode"].as_u64() == Some(code as u64)
                || bind["catch_all"].as_bool() == Some(true))
    }))
}

impl Compositor {
    pub fn detect() -> Result<Self, String> {
        // Prefer niri in a nested niri session, which may inherit Hyprland variables.
        if std::env::var_os("NIRI_SOCKET").is_some() {
            let config = if let Some(path) = std::env::var_os("NIRI_CONFIG") {
                PathBuf::from(path)
            } else {
                let home = std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
                    .ok_or("Cannot locate niri's config directory.")?;
                home.join("niri/config.kdl")
            };
            fs::metadata(&config).map_err(|e| {
                format!(
                    "Cannot open niri config {}: {e}. Set NIRI_CONFIG to the active config path.",
                    config.display()
                )
            })?;
            Ok(Self::Niri {
                config,
                installed: None,
            })
        } else if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            let version: Value = serde_json::from_str(&run("hyprctl", &["-j", "version"])?)
                .map_err(|e| e.to_string())?;
            let version = version["version"]
                .as_str()
                .ok_or("Cannot read Hyprland version.")?
                .trim_start_matches('v');
            let mut parts = version.split('.');
            let major = parts
                .next()
                .and_then(|v| v.parse::<u32>().ok())
                .ok_or("Unknown Hyprland version.")?;
            let minor = parts
                .next()
                .and_then(|v| v.parse::<u32>().ok())
                .ok_or("Unknown Hyprland version.")?;
            Ok(Self::Hyprland {
                lua: major > 0 || minor >= 55,
                key: None,
            })
        } else {
            Err("Automatic shortcuts support niri and Hyprland. Use the Start / Stop button in this session.".into())
        }
    }

    pub fn register(&mut self, key: &str) -> Result<(), String> {
        crate::shortcuts::hotkey(key)?;
        let executable = executable()?;
        match self {
            Self::Hyprland {
                lua,
                key: installed,
            } => {
                let binds: Value = serde_json::from_str(&run("hyprctl", &["-j", "binds"])?)
                    .map_err(|e| e.to_string())?;
                if hypr_conflict(&binds, key)? {
                    return Err(format!("{key} is already bound. Choose another key."));
                }
                let command = format!("{} --toggle", shell_quote(&executable));
                if *lua {
                    let script = format!(
                        "hl.bind({}, hl.dsp.exec_cmd({}), {{ description = \"Tap autoclicker\" }})",
                        serde_json::to_string(key).unwrap(),
                        serde_json::to_string(&command).unwrap()
                    );
                    hypr(&["eval", &script])?;
                } else {
                    hypr(&["keyword", "bind", &format!(", {key}, exec, {command}")])?;
                }
                *installed = Some(key.into());
            }
            Self::Niri { config, installed } => {
                let original = fs::read_to_string(&*config).map_err(|e| e.to_string())?;
                let clean = without_managed(&original)?;
                let mut keys = HashSet::new();
                collect_niri_keys(config, &clean, &mut HashSet::new(), &mut keys)?;
                if keys.contains(&key.to_ascii_lowercase()) {
                    return Err(format!("{key} is already bound. Choose another key."));
                }
                let (updated, block) = niri_binding(&clean, key, &executable)?;
                replace_niri(config, &original, &updated)?;
                *installed = Some(block);
            }
        }
        Ok(())
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        match self {
            Self::Hyprland {
                lua,
                key: Some(key),
            } => {
                // If the user reloaded/changed config, don't remove their replacement.
                let Ok(binds) = run("hyprctl", &["-j", "binds"]) else {
                    return;
                };
                let Ok(binds) = serde_json::from_str::<Value>(&binds) else {
                    return;
                };
                let Some(binds) = binds.as_array() else {
                    return;
                };
                let matching: Vec<_> = binds
                    .iter()
                    .filter(|b| b["modmask"] == 0 && b["key"].as_str() == Some(key))
                    .collect();
                let ours = if *lua {
                    matching.len() == 1 && matching[0]["description"] == "Tap autoclicker"
                } else {
                    matching.len() == 1
                        && executable().is_ok_and(|exe| {
                            matching[0]["arg"].as_str()
                                == Some(&format!("{} --toggle", shell_quote(&exe)))
                        })
                };
                if ours {
                    if *lua {
                        let _ = hypr(&[
                            "eval",
                            &format!("hl.unbind({})", serde_json::to_string(key).unwrap()),
                        ]);
                    } else {
                        let _ = hypr(&["keyword", "unbind", &format!(", {key}")]);
                    }
                }
            }
            Self::Niri {
                config,
                installed: Some(block),
            } => {
                if let Ok(original) = fs::read_to_string(&*config)
                    && original.contains(block.as_str())
                    && let Ok(clean) = without_managed(&original)
                {
                    let _ = replace_niri(config, &original, &clean);
                }
            }
            _ => {}
        }
    }
}

fn without_managed(text: &str) -> Result<String, String> {
    match (text.find(BEGIN), text.find(END)) {
        (None, None) => Ok(text.into()),
        (Some(start), Some(end)) if end > start && !text[end + END.len()..].contains(BEGIN) => {
            Ok(format!("{}{}", &text[..start], &text[end + END.len()..]))
        }
        _ => Err("Tap's niri shortcut block is damaged. Restore it before retrying.".into()),
    }
}

// Insert into an existing binds section so this also works before niri 25.11,
// which rejected multiple top-level binds sections. Keep every original byte.
fn niri_binding(clean: &str, key: &str, executable: &str) -> Result<(String, String), String> {
    let document: kdl::KdlDocument = clean
        .parse()
        .map_err(|e| format!("Cannot read niri config: {e}"))?;
    let binding = format!(
        "    {key} repeat=false {{ spawn {} \"--toggle\"; }}\n",
        serde_json::to_string(executable).unwrap()
    );
    let (offset, body) = if let Some(children) = document.get("binds").and_then(|n| n.children()) {
        (children.span().offset(), binding)
    } else {
        (clean.len(), format!("binds {{\n{binding}}}\n"))
    };
    let block = format!("{BEGIN}{body}{END}");
    Ok((
        format!("{}{}{}", &clean[..offset], block, &clean[offset..]),
        block,
    ))
}

fn collect_niri_keys(
    path: &Path,
    text: &str,
    visited: &mut HashSet<PathBuf>,
    keys: &mut HashSet<String>,
) -> Result<(), String> {
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    if !visited.insert(canonical.clone()) {
        return Err("Cannot inspect cyclic or repeated niri includes safely.".into());
    }
    let document: kdl::KdlDocument = text
        .parse()
        .map_err(|e| format!("Cannot read niri bindings: {e}"))?;
    for node in document.nodes() {
        match node.name().value() {
            "binds" => {
                if let Some(children) = node.children() {
                    for bind in children.nodes() {
                        keys.insert(bind.name().value().to_ascii_lowercase());
                    }
                }
            }
            "include" => {
                let name = node
                    .get(0)
                    .and_then(|e| e.value().as_string())
                    .ok_or("Cannot read niri include path.")?;
                let include = if let Some(rest) = name.strip_prefix("~/") {
                    PathBuf::from(
                        std::env::var_os("HOME").ok_or("HOME is required for niri includes.")?,
                    )
                    .join(rest)
                } else {
                    path.parent().unwrap().join(name)
                };
                let contents = match fs::read_to_string(&include) {
                    Ok(contents) => contents,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::NotFound
                            && node.get("optional").and_then(|e| e.value().as_bool())
                                == Some(true) =>
                    {
                        continue;
                    }
                    Err(e) => return Err(format!("Cannot inspect {}: {e}", include.display())),
                };
                collect_niri_keys(&include, &contents, visited, keys)?;
            }
            _ => {}
        }
    }
    visited.remove(&canonical);
    Ok(())
}

fn replace_niri(path: &Path, original: &str, updated: &str) -> Result<(), String> {
    replace_niri_validated(path, original, updated, |temp| {
        run(
            "niri",
            &[
                "validate",
                "--config",
                temp.to_str().ok_or("Invalid niri config path")?,
            ],
        )
        .map(|_| ())
    })
}

fn replace_niri_validated(
    path: &Path,
    original: &str,
    updated: &str,
    validate: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    use std::io::Write;
    // Validate alongside the original so relative includes keep their meaning.
    let target = path.canonicalize().map_err(|e| e.to_string())?;
    let temp = path.with_file_name(format!(".tap-shortcut-{}.kdl", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        file.set_permissions(fs::metadata(path).map_err(|e| e.to_string())?.permissions())
            .map_err(|e| e.to_string())?;
        file.write_all(updated.as_bytes())
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        validate(&temp)?;
        if fs::read_to_string(path).map_err(|e| e.to_string())? != original {
            return Err("niri config changed while checking shortcuts. Try again.".into());
        }
        fs::rename(&temp, &target).map_err(|e| e.to_string())
    })();
    let _ = fs::remove_file(temp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hyprland_detects_symbols_keycodes_and_catch_all_without_blocking_modified_keys() {
        let binds = serde_json::json!([
            {"modmask":64,"key":"F6"}, {"modmask":0,"key":"f7"},
            {"modmask":0,"keycode":74}, {"modmask":0,"keycode":95}
        ]);
        assert!(!hypr_conflict(&binds, "F6").unwrap());
        for key in ["F7", "F8", "F11"] {
            assert!(hypr_conflict(&binds, key).unwrap());
        }
        assert!(hypr_conflict(&serde_json::json!([{"modmask":0,"catch_all":true}]), "F6").unwrap());
        assert!(hypr_conflict(&Value::Null, "F6").is_err());
    }
    #[test]
    fn managed_block_removal_preserves_other_settings_and_rejects_damage() {
        let block = format!("{BEGIN}binds {{ F6 {{ spawn \"tap\"; }} }}\n{END}");
        let text = format!("// user settings\n{block}// more user settings\n");
        assert_eq!(
            without_managed(&text).unwrap(),
            "// user settings\n// more user settings\n"
        );
        assert!(without_managed(BEGIN).is_err());
        assert!(without_managed(&format!("{block}{block}")).is_err());
    }
    #[test]
    fn niri_inspection_includes_files_ignores_comments_and_modified_keys() {
        let dir = std::env::temp_dir().join(format!("tap-niri-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.kdl");
        fs::write(&config, "").unwrap();
        fs::write(dir.join("keys.kdl"), "binds { F7 { spawn \"other\"; }; }\n").unwrap();
        let text =
            "// F6 { fake; }\nbinds { Mod+F6 { spawn \"other\"; }; }\ninclude \"keys.kdl\"\n";
        let mut keys = HashSet::new();
        collect_niri_keys(&config, text, &mut HashSet::new(), &mut keys).unwrap();
        assert!(!keys.contains("f6"));
        assert!(keys.contains("f7"));
        fs::write(dir.join("keys.kdl"), "include \"config.kdl\"\n").unwrap();
        assert!(collect_niri_keys(&config, text, &mut HashSet::new(), &mut keys).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn niri_insertion_preserves_original_bytes_and_uses_existing_binds() {
        for original in [
            "binds { F7 { spawn \"other\"; }; }\n",
            "// comment\n",
            "binds { /* braces { } */ }\n",
        ] {
            let (updated, _) = niri_binding(original, "F6", "/a b/tap").unwrap();
            let doc: kdl::KdlDocument = updated.parse().unwrap();
            assert_eq!(
                doc.nodes()
                    .iter()
                    .filter(|n| n.name().value() == "binds")
                    .count(),
                1
            );
            assert!(
                doc.get("binds")
                    .unwrap()
                    .children()
                    .unwrap()
                    .get("F6")
                    .is_some()
            );
            assert_eq!(without_managed(&updated).unwrap(), original);
        }
    }
    #[test]
    fn niri_writes_only_validated_changes_and_preserves_concurrent_edits() {
        let dir = std::env::temp_dir().join(format!("tap-niri-write-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.kdl");
        let original = "// original\n";
        fs::write(&config, original).unwrap();
        assert!(
            replace_niri_validated(&config, original, "bad", |_| Err("invalid config".into()))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
        assert!(
            replace_niri_validated(&config, original, "replacement", |_| {
                fs::write(&config, "// concurrent edit\n").unwrap();
                Ok(())
            })
            .is_err()
        );
        assert_eq!(fs::read_to_string(&config).unwrap(), "// concurrent edit\n");
        fs::write(&config, original).unwrap();
        replace_niri_validated(&config, original, "// valid\n", |candidate| {
            assert_eq!(fs::read_to_string(candidate).unwrap(), "// valid\n");
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read_to_string(&config).unwrap(), "// valid\n");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn niri_writes_preserve_config_symlinks() {
        let dir = std::env::temp_dir().join(format!("tap-niri-link-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("dotfiles.kdl");
        let link = dir.join("config.kdl");
        fs::write(&target, "// original\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        replace_niri_validated(&link, "// original\n", "// updated\n", |_| Ok(())).unwrap();
        assert!(link.is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "// updated\n");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn executable_shell_quoting_handles_spaces_and_quotes() {
        assert_eq!(shell_quote("/a b/it's tap"), "'/a b/it'\\''s tap'");
    }
}
