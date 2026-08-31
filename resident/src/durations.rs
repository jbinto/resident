use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result, bail};
use resident_core::DurationUpdate;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurationLine {
    key: String,
    duration_seconds: f64,
}

pub fn load(path: &Path) -> Result<Vec<DurationUpdate>> {
    let file =
        File::open(path).with_context(|| format!("open duration metadata {}", path.display()))?;
    let mut updates = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line_number = index + 1;
        let line = line
            .with_context(|| format!("read duration metadata {}:{line_number}", path.display()))?;
        if line.trim().is_empty() {
            bail!(
                "duration metadata {}:{line_number} is blank",
                path.display()
            );
        }
        let row: DurationLine = serde_json::from_str(&line)
            .with_context(|| format!("parse duration metadata {}:{line_number}", path.display()))?;
        updates.push(DurationUpdate {
            key: row.key,
            duration_seconds: row.duration_seconds,
        });
    }
    if updates.is_empty() {
        bail!("duration metadata {} is empty", path.display());
    }
    Ok(updates)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn parses_strict_json_lines_with_context() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "{{\"key\":\"a\",\"duration_seconds\":12.5}}").unwrap();
        let found = load(file.path()).unwrap();
        assert_eq!(
            found,
            vec![DurationUpdate {
                key: "a".into(),
                duration_seconds: 12.5,
            }]
        );

        let mut bad = tempfile::NamedTempFile::new().unwrap();
        writeln!(bad, "{{\"key\":\"a\",\"duration\":12.5}}").unwrap();
        let error = load(bad.path()).unwrap_err().to_string();
        assert!(error.contains(":1"), "{error}");
    }
}
