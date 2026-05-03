//! Strict CSV reader for smelt seed files.
//!
//! Strict defaults per `seeds.md §"CSV format"`:
//! - Comma delimiter (`b','`)
//! - Double-quote quoting (`b'"'`, `""` for embedded `"`)
//! - First row is the header (required)
//! - Empty cell → `None` (NULL)
//! - UTF-8 BOM (`\xEF\xBB\xBF`) consumed silently
//! - LF and CRLF line endings, mixed accepted
//! - Tab-delimited files → `SeedError::WrongDelimiter`
//! - Non-conforming files → `SeedError::CsvParseError` with file/line/message

pub use super::error::SeedError;
use csv::ReaderBuilder;
use std::path::Path;

/// A single parsed data row from a CSV seed file.
#[derive(Debug, Clone, PartialEq)]
pub struct CsvRecord {
    /// 0-based index of this data row (after the header row).
    pub row_index: usize,
    /// Parsed field values. `None` means the cell was empty (NULL).
    pub fields: Vec<Option<String>>,
}

/// Read a CSV file with strict defaults and return `(headers, row_iterator)`.
///
/// The iterator yields `Result<CsvRecord, SeedError>`. Callers should collect
/// all rows and propagate the first error encountered.
///
/// Spec rules enforced here:
/// - UTF-8 BOM stripped from the first header field.
/// - Comma delimiter required. If the header line contains a tab but no comma
///   (other than inside quotes), a `WrongDelimiter` error is returned immediately.
/// - CRLF/LF normalised by the `csv` crate.
pub fn read_csv(
    path: &Path,
) -> Result<
    (
        Vec<String>,
        impl Iterator<Item = Result<CsvRecord, SeedError>>,
    ),
    SeedError,
> {
    let path_buf = path.to_path_buf();

    // Read the raw bytes first so we can strip the BOM before passing to csv.
    let raw = std::fs::read(path).map_err(|e| SeedError::Io {
        path: path_buf.clone(),
        message: e.to_string(),
    })?;

    // Strip UTF-8 BOM if present.
    let data = if raw.starts_with(b"\xEF\xBB\xBF") {
        raw[3..].to_vec()
    } else {
        raw
    };

    // Check for wrong delimiter: if the header line contains a tab (\t) but
    // does not contain a comma (outside of quotes), the file is likely
    // tab-delimited. We do a quick scan of the first line to detect this.
    detect_wrong_delimiter(&data, &path_buf)?;

    // Build a strict comma-delimited reader.
    let mut reader = ReaderBuilder::new()
        .delimiter(b',')
        .quote(b'"')
        .has_headers(true)
        .flexible(false)
        .from_reader(data.as_slice());

    // Extract headers (first row).
    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| SeedError::CsvParseError {
            path: path_buf.clone(),
            line: 1,
            message: e.to_string(),
        })?
        .iter()
        .map(|h| h.to_string())
        .collect();

    if headers.is_empty() {
        return Err(SeedError::CsvParseError {
            path: path_buf.clone(),
            line: 1,
            message: "CSV file has no header row".to_string(),
        });
    }

    // We've consumed the reader's headers; now we need to iterate data rows.
    // Because the iterator borrows the reader, we must collect all records.
    let path_for_iter = path_buf.clone();
    let mut records: Vec<Result<CsvRecord, SeedError>> = Vec::new();
    let mut row_index = 0usize;

    for result in reader.records() {
        match result {
            Err(e) => {
                let line = e
                    .position()
                    .map(|p| p.line() as usize)
                    .unwrap_or(row_index + 2);
                records.push(Err(SeedError::CsvParseError {
                    path: path_for_iter.clone(),
                    line,
                    message: e.to_string(),
                }));
            }
            Ok(record) => {
                let fields: Vec<Option<String>> = record
                    .iter()
                    .map(|field| {
                        if field.is_empty() {
                            None
                        } else {
                            Some(field.to_string())
                        }
                    })
                    .collect();
                records.push(Ok(CsvRecord { row_index, fields }));
                row_index += 1;
            }
        }
    }

    Ok((headers, records.into_iter()))
}

/// Scan the first line of raw CSV bytes to detect tab-delimited files.
///
/// The check is intentionally simple: if the first line (up to the first LF
/// or CRLF) contains a `\t` and no `,` outside of double-quoted regions, it's
/// very likely a tab-delimited file and we return `WrongDelimiter`.
fn detect_wrong_delimiter(data: &[u8], path: &Path) -> Result<(), SeedError> {
    // Find end of first line.
    let first_line_end = data.iter().position(|&b| b == b'\n').unwrap_or(data.len());
    let first_line = &data[..first_line_end];
    // Strip trailing CR.
    let first_line = first_line.strip_suffix(b"\r").unwrap_or(first_line);

    let has_tab = first_line.contains(&b'\t');
    if !has_tab {
        return Ok(());
    }

    // Count commas outside quotes in the first line.
    let mut in_quote = false;
    let mut comma_count = 0usize;
    let mut i = 0;
    while i < first_line.len() {
        match first_line[i] {
            b'"' => {
                in_quote = !in_quote;
                // Handle escaped quote ("").
                if in_quote && i + 1 < first_line.len() && first_line[i + 1] == b'"' {
                    i += 1; // skip the second quote
                }
            }
            b',' if !in_quote => comma_count += 1,
            _ => {}
        }
        i += 1;
    }

    // If no commas outside quotes, but there are tabs, it's wrong-delimiter.
    if comma_count == 0 {
        // Find the first tab char for the error message.
        let found = '\t';
        return Err(SeedError::WrongDelimiter {
            path: path.to_path_buf(),
            found,
        });
    }

    Ok(())
}
