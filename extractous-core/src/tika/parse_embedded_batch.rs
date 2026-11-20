use crate::embedded::{EmbeddedDocument, EmbeddedExtractResult};
use crate::errors::ExtractResult;
use crate::tika::parse_embedded_optimized::extract_embedded_optimized;
use crate::{OfficeParserConfig, PdfParserConfig, TesseractOcrConfig};

/// Batch embedded document extraction
/// Since the Java side currently implements batch as calling optimized with a limit,
/// we'll do the same here for consistency
pub fn extract_embedded_batch(
    file_path: &str,
    pdf_conf: &PdfParserConfig,
    office_conf: &OfficeParserConfig,
    ocr_conf: &TesseractOcrConfig,
) -> ExtractResult<EmbeddedExtractResult> {
    // For now, use the optimized extraction which handles all documents
    // In the future, this could be enhanced to support actual batching with offset/limit
    extract_embedded_optimized(file_path, pdf_conf, office_conf, ocr_conf)
}

/// Streaming embedded document extraction with callback
/// Processes embedded documents in batches to reduce memory usage
pub fn extract_embedded_streaming<F>(
    file_path: &str,
    pdf_conf: &PdfParserConfig,
    office_conf: &OfficeParserConfig,
    ocr_conf: &TesseractOcrConfig,
    batch_size: usize,
    mut callback: F,
) -> ExtractResult<()>
where
    F: FnMut(Vec<EmbeddedDocument>) -> ExtractResult<bool>,
{
    // For now, we'll extract all documents and then batch them
    // In the future, this should be implemented with proper streaming from Java side
    let result = extract_embedded_optimized(file_path, pdf_conf, office_conf, ocr_conf)?;
    
    // Process documents in batches
    let mut batch = Vec::with_capacity(batch_size);
    
    for doc in result.documents {
        batch.push(doc);
        
        if batch.len() >= batch_size {
            // Call the callback with the current batch
            let continue_processing = callback(batch)?;
            if !continue_processing {
                return Ok(());
            }
            batch = Vec::with_capacity(batch_size);
        }
    }
    
    // Process any remaining documents
    if !batch.is_empty() {
        callback(batch)?;
    }
    
    Ok(())
}