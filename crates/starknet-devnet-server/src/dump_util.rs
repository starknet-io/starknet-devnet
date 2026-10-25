use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use starknet_core::error::{DevnetResult, Error};

/// Writes `dump` to `path`, replacing the file only once the new content is fully written and
/// synced. If writing fails, an existing file at `path` is left intact.
pub fn write_dump(dump: &impl serde::Serialize, path: &str) -> DevnetResult<()> {
    let destination = Path::new(path);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::UnexpectedInternalError { msg: error.to_string() })?
        .as_nanos();
    let temporary = destination.with_extension(format!("{nonce}.tmp"));

    let result = (|| {
        let mut writer = BufWriter::new(File::create(&temporary)?);
        serde_json::to_writer(&mut writer, dump)
            .map_err(|error| Error::SerializationError { origin: error.to_string() })?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        fs::rename(&temporary, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Reads the dump at `path` as JSON, without interpreting it.
pub fn read_dump(path: &str) -> DevnetResult<serde_json::Value> {
    let file = File::open(Path::new(path)).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => Error::FileNotFound,
        _ => Error::IoError(error),
    })?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|e| Error::DeserializationError { origin: e.to_string() })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{read_dump, write_dump};

    #[test]
    fn writes_and_reads_dump() {
        let directory = std::env::temp_dir().join(format!("devnet-dump-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("dump.json");
        let path = path.to_str().unwrap();

        write_dump(&json!({ "a": 1 }), path).unwrap();
        write_dump(&json!({ "a": 2 }), path).unwrap();
        assert_eq!(read_dump(path).unwrap(), json!({ "a": 2 }));
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn failed_write_keeps_previous_dump() {
        let directory =
            std::env::temp_dir().join(format!("devnet-dump-failure-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("dump.json");
        let path = path.to_str().unwrap();
        write_dump(&json!({ "a": 1 }), path).unwrap();

        // Serialization fails midway, after the temporary file was created.
        struct Unserializable;
        impl serde::Serialize for Unserializable {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("unserializable"))
            }
        }
        assert!(write_dump(&Unserializable, path).is_err());

        assert_eq!(read_dump(path).unwrap(), json!({ "a": 1 }));
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
