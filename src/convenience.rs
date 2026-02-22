//! Convenience function for end-to-end PDF generation

use std::fmt;
use std::io::BufRead;

use age::secrecy::SecretString;

use crate::builder;
use crate::encryption;
use crate::page::PageSize;

/// Errors that can occur during PDF generation
#[derive(Debug)]
pub enum PaperAgeError {
    /// The plaintext data could not be encrypted
    Encryption(String),
    /// The PDF document could not be initialized
    DocumentInit(String),
    /// The PDF could not be created (e.g. QR code too large)
    PdfCreation(String),
}

impl fmt::Display for PaperAgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaperAgeError::Encryption(msg) => write!(f, "Encryption failed: {msg}"),
            PaperAgeError::DocumentInit(msg) => write!(f, "Document initialization failed: {msg}"),
            PaperAgeError::PdfCreation(msg) => write!(f, "PDF creation failed: {msg}"),
        }
    }
}

impl std::error::Error for PaperAgeError {}

/// Generate a PaperAge PDF from plaintext data and a passphrase.
///
/// This is a high-level convenience function that handles encryption and PDF
/// generation in a single call.
///
/// # Arguments
///
/// * `title` - The document title (appears in the PDF and its metadata)
/// * `data` - A buffered reader providing the plaintext data to encrypt
/// * `passphrase` - The passphrase used to encrypt the data
/// * `notes_label` - Label for the notes field (defaults to `"Passphrase:"`)
/// * `skip_notes_line` - Whether to omit the notes placeholder line (defaults to `false`)
/// * `page_size` - The page size to use (defaults to [`PageSize::A4`])
/// * `grid` - Whether to draw a debug grid on the page (defaults to `false`)
///
/// # Returns
///
/// The PDF file contents as a `Vec<u8>`, or a [`PaperAgeError`] describing
/// what went wrong.
///
/// # Example
///
/// ```no_run
/// use paper_age::convenience::create_pdf;
/// use paper_age::page::PageSize;
///
/// let pdf_bytes = create_pdf(
///     "My Secret".to_string(),
///     &mut &b"secret data to encrypt"[..],
///     "hunter2",
///     None,
///     None,
///     None,
///     None,
/// ).expect("PDF generation failed");
/// ```
pub fn create_pdf(
    title: String,
    data: &mut dyn BufRead,
    passphrase: &str,
    notes_label: Option<String>,
    skip_notes_line: Option<bool>,
    page_size: Option<PageSize>,
    grid: Option<bool>,
) -> Result<Vec<u8>, PaperAgeError> {
    let notes_label = notes_label.unwrap_or_else(|| "Passphrase:".to_string());
    let skip_notes_line = skip_notes_line.unwrap_or(false);
    let page_size = page_size.unwrap_or(PageSize::A4);
    let grid = grid.unwrap_or(false);

    let passphrase_secret = SecretString::from(passphrase.to_owned());

    let (_plaintext_len, encrypted) = encryption::encrypt_plaintext(data, passphrase_secret)
        .map_err(|e| PaperAgeError::Encryption(e.to_string()))?;

    let pdf = builder::Document::new(title, page_size)
        .map_err(|e| PaperAgeError::DocumentInit(e.to_string()))?;

    let bytes = pdf
        .create_pdf(grid, notes_label, skip_notes_line, encrypted)
        .map_err(|e| PaperAgeError::PdfCreation(e.to_string()))?;

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_pdf_defaults() {
        let result = create_pdf(
            "Test Document".to_string(),
            &mut &b"hello world"[..],
            "passphrase",
            None,
            None,
            None,
            None,
        );
        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_create_pdf_with_options() {
        let result = create_pdf(
            "Custom Document".to_string(),
            &mut &b"secret data"[..],
            "hunter2",
            Some("Recovery key:".to_string()),
            Some(true),
            Some(PageSize::Letter),
            Some(true),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_pdf_empty_data() {
        let result = create_pdf(
            "Empty".to_string(),
            &mut &b""[..],
            "passphrase",
            None,
            None,
            None,
            None,
        );
        assert!(result.is_ok());
    }
}
