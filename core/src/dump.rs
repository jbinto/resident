use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use crate::config::{MAX_FREQUENCY_BIN, MAX_HASH};
use crate::{Error, Fingerprint, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct ResourceMeta {
    pub source_id: String,
    pub key: String,
    pub duration: f64,
    pub declared_prints: u64,
}

#[derive(Clone, Debug)]
pub struct DumpResource {
    pub meta: ResourceMeta,
    pub prints: Vec<Fingerprint>,
    pub prints_path: PathBuf,
}

pub fn load_dump_dir(path: &Path) -> Result<Vec<DumpResource>> {
    let entries = fs::read_dir(path).map_err(|source| Error::io(path, source))?;
    let mut metadata_paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| Error::io(path, source))?;
        let entry_path = entry.path();
        if entry_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("_meta_data.txt"))
        {
            metadata_paths.push(entry_path);
        }
    }
    metadata_paths.sort();
    if metadata_paths.is_empty() {
        return Err(Error::BadRequest(format!(
            "dump directory {} contains no metadata files",
            path.display()
        )));
    }

    metadata_paths
        .into_iter()
        .map(|meta_path| {
            let meta = load_metadata(&meta_path)?;
            let plain = path.join(format!("{}.tdb", meta.source_id));
            let compressed = path.join(format!("{}.tdb.zst", meta.source_id));
            let prints_path = if plain.is_file() {
                plain
            } else if compressed.is_file() {
                compressed
            } else {
                return Err(Error::BadRequest(format!(
                    "metadata {} has no matching .tdb or .tdb.zst",
                    meta_path.display()
                )));
            };
            let prints = load_prints(&prints_path)?;
            if prints.len() as u64 != meta.declared_prints {
                return Err(Error::InvalidDump {
                    path: prints_path.clone(),
                    line: 0,
                    message: format!(
                        "metadata declares {} prints, file contains {}",
                        meta.declared_prints,
                        prints.len()
                    ),
                });
            }
            Ok(DumpResource {
                meta,
                prints,
                prints_path,
            })
        })
        .collect()
}

pub fn load_metadata(path: &Path) -> Result<ResourceMeta> {
    let text = fs::read_to_string(path).map_err(|source| Error::io(path, source))?;
    let lines: Vec<_> = text.lines().collect();
    if lines.len() != 4 {
        return Err(Error::InvalidDump {
            path: path.to_path_buf(),
            line: 0,
            message: format!(
                "metadata must contain exactly four lines, got {}",
                lines.len()
            ),
        });
    }
    let source_id = parse_value::<u32>(path, 1, lines[0], "resource id")?.to_string();
    let duration = parse_value::<f64>(path, 2, lines[1], "duration")?;
    if !duration.is_finite() || duration < 0.0 {
        return Err(invalid(path, 2, "duration must be finite and non-negative"));
    }
    let declared_prints = parse_value::<u64>(path, 3, lines[2], "print count")?;
    let key = lines[3].to_owned();
    if key.is_empty() {
        return Err(invalid(path, 4, "resource key must not be empty"));
    }
    Ok(ResourceMeta {
        source_id,
        key,
        duration,
        declared_prints,
    })
}

pub fn load_prints(path: &Path) -> Result<Vec<Fingerprint>> {
    let file = File::open(path).map_err(|source| Error::io(path, source))?;
    if path.extension().is_some_and(|extension| extension == "zst") {
        let decoder =
            zstd::stream::read::Decoder::new(file).map_err(|source| Error::io(path, source))?;
        parse_prints(path, decoder)
    } else {
        parse_prints(path, file)
    }
}

fn parse_prints(path: &Path, reader: impl Read) -> Result<Vec<Fingerprint>> {
    let mut prints = Vec::new();
    for (index, line) in BufReader::new(reader).lines().enumerate() {
        let number = index + 1;
        let line = line.map_err(|source| Error::io(path, source))?;
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 4 {
            return Err(invalid(
                path,
                number,
                format!("expected four fields, got {}", fields.len()),
            ));
        }
        let hash = parse_value::<u64>(path, number, fields[0], "hash")?;
        if hash > MAX_HASH {
            return Err(invalid(
                path,
                number,
                format!("hash {hash} exceeds 34 bits"),
            ));
        }
        let t = parse_value::<u32>(path, number, fields[2], "time bin")?;
        let f = parse_value::<u16>(path, number, fields[3], "frequency bin")?;
        if f > MAX_FREQUENCY_BIN {
            return Err(invalid(
                path,
                number,
                format!("frequency bin {f} exceeds {MAX_FREQUENCY_BIN}"),
            ));
        }
        prints.push(Fingerprint::new(hash, t, f));
    }
    Ok(prints)
}

fn parse_value<T: std::str::FromStr>(
    path: &Path,
    line: usize,
    value: &str,
    description: &str,
) -> Result<T> {
    value
        .parse()
        .map_err(|_| invalid(path, line, format!("invalid {description}: {value:?}")))
}

fn invalid(path: &Path, line: usize, message: impl Into<String>) -> Error {
    Error::InvalidDump {
        path: path.to_path_buf(),
        line,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn parses_hashes_above_u32() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "6782338376 1098801918 15 309 ").unwrap();
        assert_eq!(
            load_prints(file.path()).unwrap(),
            vec![Fingerprint::new(6_782_338_376, 15, 309)]
        );
    }

    #[test]
    fn rejects_out_of_range_values_with_line_context() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "17179869184 1 2 3").unwrap();
        let error = load_prints(file.path()).unwrap_err().to_string();
        assert!(error.contains(":1:"));
        assert!(error.contains("exceeds 34 bits"));
    }
}
