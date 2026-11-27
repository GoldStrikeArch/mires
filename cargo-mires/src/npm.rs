//! Generates an npm `package.json` for compiled Mires artifacts (`--npm`).

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::cli::OutputFormat;

/// Inputs describing the emitted package artifacts inside `--out-dir`.
#[derive(Clone, Debug)]
pub struct NpmPackageInput<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub description: Option<&'a str>,
    pub license: Option<&'a str>,
    pub format: OutputFormat,
    pub entry_file: &'a str,
    pub dts_file: Option<&'a str>,
    pub files: &'a [String],
}

#[derive(Serialize)]
struct PackageJson<'a> {
    name: &'a str,
    version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    license: Option<&'a str>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pkg_type: Option<&'a str>,
    main: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    types: Option<String>,
    exports: ExportsMap,
    files: Vec<String>,
}

#[derive(Serialize)]
struct ExportsMap {
    #[serde(rename = ".")]
    root: ExportConditions,
}

/// Conditional exports for `"."`.
///
/// `types` is placed first because TypeScript's `node16`/`bundler` module resolution
/// requires the `"types"` condition to precede `"import"` / `"default"`.
#[derive(Serialize)]
struct ExportConditions {
    #[serde(skip_serializing_if = "Option::is_none")]
    types: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    import: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<String>,
}

/// Render a deterministic, pretty-printed `package.json` string (with trailing newline).
pub fn render_package_json(input: &NpmPackageInput<'_>) -> Result<String, String> {
    let entry_rel = dot_slash(input.entry_file);
    let dts_rel = input.dts_file.map(dot_slash);

    let (pkg_type, export_conditions) = match input.format {
        OutputFormat::Esm => (
            Some("module"),
            ExportConditions {
                types: dts_rel.clone(),
                import: Some(entry_rel.clone()),
                default: None,
            },
        ),
        OutputFormat::Script => (
            None,
            ExportConditions {
                types: dts_rel.clone(),
                import: None,
                default: Some(entry_rel.clone()),
            },
        ),
    };

    let mut files_set: BTreeSet<String> = input
        .files
        .iter()
        .map(|f| f.trim_start_matches("./").to_owned())
        .filter(|f| !f.is_empty() && f != "package.json")
        .collect();
    files_set.insert(input.entry_file.trim_start_matches("./").to_owned());
    if let Some(dts) = input.dts_file {
        files_set.insert(dts.trim_start_matches("./").to_owned());
    }

    let manifest = PackageJson {
        name: input.name,
        version: input.version,
        description: input.description,
        license: input.license,
        pkg_type,
        main: entry_rel,
        types: dts_rel,
        exports: ExportsMap {
            root: export_conditions,
        },
        files: files_set.into_iter().collect(),
    };

    let mut json = serde_json::to_string_pretty(&manifest)
        .map_err(|err| format!("failed to serialize `package.json`: {err}"))?;
    json.push('\n');
    Ok(json)
}

/// Write `package.json` inside `out_dir`, returning its path.
pub fn write_package_json(
    out_dir: &Path,
    input: &NpmPackageInput<'_>,
) -> Result<std::path::PathBuf, String> {
    let json = render_package_json(input)?;
    let path = out_dir.join("package.json");
    std::fs::write(&path, json.as_bytes())
        .map_err(|err| format!("failed to write `{}`: {err}", path.display()))?;
    Ok(path)
}

fn dot_slash(path: &str) -> String {
    let stripped = path.trim_start_matches("./");
    format!("./{stripped}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_esm_package_json_with_types_before_import() {
        let files = vec![
            "my_lib.mjs".to_owned(),
            "my_lib.mjs.map".to_owned(),
            "my_lib.d.ts".to_owned(),
            "shim.mjs".to_owned(),
        ];
        let rendered = render_package_json(&NpmPackageInput {
            name: "my-lib",
            version: "1.2.3",
            description: Some("Compiled with Mires"),
            license: Some("MIT"),
            format: OutputFormat::Esm,
            entry_file: "my_lib.mjs",
            dts_file: Some("my_lib.d.ts"),
            files: &files,
        })
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed["name"], "my-lib");
        assert_eq!(parsed["version"], "1.2.3");
        assert_eq!(parsed["description"], "Compiled with Mires");
        assert_eq!(parsed["license"], "MIT");
        assert_eq!(parsed["type"], "module");
        assert_eq!(parsed["main"], "./my_lib.mjs");
        assert_eq!(parsed["types"], "./my_lib.d.ts");
        assert_eq!(parsed["exports"]["."]["types"], "./my_lib.d.ts");
        assert_eq!(parsed["exports"]["."]["import"], "./my_lib.mjs");
        assert_eq!(
            parsed["files"],
            serde_json::json!(["my_lib.d.ts", "my_lib.mjs", "my_lib.mjs.map", "shim.mjs"])
        );

        let types_pos = rendered.find("\"types\": \"./my_lib.d.ts\"").unwrap();
        let import_pos = rendered.find("\"import\": \"./my_lib.mjs\"").unwrap();
        assert!(types_pos < import_pos);
    }

    #[test]
    fn renders_script_package_json_without_module_type_or_optional_fields() {
        let files = vec!["app.js".to_owned()];
        let rendered = render_package_json(&NpmPackageInput {
            name: "app",
            version: "0.1.0",
            description: None,
            license: None,
            format: OutputFormat::Script,
            entry_file: "app.js",
            dts_file: None,
            files: &files,
        })
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert!(parsed.get("type").is_none());
        assert!(parsed.get("description").is_none());
        assert!(parsed.get("license").is_none());
        assert!(parsed.get("types").is_none());
        assert_eq!(parsed["main"], "./app.js");
        assert_eq!(parsed["exports"]["."]["default"], "./app.js");
        assert!(parsed["exports"]["."].get("import").is_none());
    }
}
