use crate::embedded::{EmbeddedDocument, EmbeddedExtractResult};
use crate::errors::{Error, ExtractResult};
use crate::tika::jni_utils::{jni_call_static_method, jni_new_string_as_jvalue};
use crate::tika::vm;
use crate::tika::wrappers::{JOptimizedResult, JOfficeParserConfig, JPDFParserConfig, JTesseractOcrConfig};
use crate::{OfficeParserConfig, PdfParserConfig, TesseractOcrConfig};
use jni::AttachGuard;
use std::collections::HashMap;
use std::io::{Cursor, Read};

/// Attach the calling thread for the duration of this call.
///
/// NOTE: `attach_current_thread_as_daemon` (leaving Rayon workers permanently
/// attached) was measured to cost +62% CPU for identical work at 12 threads —
/// GraalVM CE only ships the stop-the-world Serial collector, which must suspend
/// every attached thread at each collection, so keeping all workers attached
/// lengthens every pause. The per-call attach/detach is cheaper than that.
///
/// The `AttachGuard` detaches on drop, which also implicitly reclaims JNI local
/// references. The explicit `AutoLocal` scoping in `wrappers.rs` is kept anyway:
/// it bounds the local-reference table *within* a single call, which is what
/// stops all ~1000 extracted images being pinned in the Java heap at once.
fn get_vm_attach_current_thread<'local>() -> ExtractResult<AttachGuard<'local>> {
    let env = vm().attach_current_thread()?;
    Ok(env)
}

/// Optimized embedded document extraction that minimizes JNI overhead
pub fn extract_embedded_optimized(
    file_path: &str,
    pdf_conf: &PdfParserConfig,
    office_conf: &OfficeParserConfig,
    ocr_conf: &TesseractOcrConfig,
) -> ExtractResult<EmbeddedExtractResult> {
    let mut env = get_vm_attach_current_thread()?;
    
    // Create Java string for file path
    let file_path_val = jni_new_string_as_jvalue(&mut env, file_path)?;
    
    // Create Java config objects
    let j_pdf_conf = JPDFParserConfig::new(&mut env, pdf_conf)?;
    let j_office_conf = JOfficeParserConfig::new(&mut env, office_conf)?;
    let j_ocr_conf = JTesseractOcrConfig::new(&mut env, ocr_conf)?;
    
    // Call the optimized Java method
    let result = jni_call_static_method(
        &mut env,
        "ai/yobix/TikaNativeMain",
        "extractEmbeddedOptimized",
        "(Ljava/lang/String;\
        Lorg/apache/tika/parser/pdf/PDFParserConfig;\
        Lorg/apache/tika/parser/microsoft/OfficeParserConfig;\
        Lorg/apache/tika/parser/ocr/TesseractOCRConfig;\
        )Lai/yobix/OptimizedEmbeddedExtractor$OptimizedResult;",
        &[
            (&file_path_val).into(),
            (&j_pdf_conf.internal).into(),
            (&j_office_conf.internal).into(),
            (&j_ocr_conf.internal).into(),
        ],
    )?;
    
    let result_obj = result.l()?;
    
    // Convert Java result to Rust
    let j_result = JOptimizedResult::new(&mut env, result_obj)?;
    
    // Check for errors
    if j_result.error_code != 0 {
        let error_msg = j_result.error_message.unwrap_or_else(|| {
            format!("Optimized embedded extraction failed with code {}", j_result.error_code)
        });
        return Err(Error::ParseError(error_msg));
    }
    
    // Unpack the optimized data
    let packed_data = j_result.packed_data.ok_or_else(|| {
        Error::ParseError("No packed data returned from optimized extraction".to_string())
    })?;
    
    // Parse the packed data format
    // Format: [count][doc1_size][doc1_data][doc2_size][doc2_data]...
    // Where each doc_data contains: [name_len][name][type_len][type][rel_id_len][rel_id][content_len][content]
    let documents = unpack_optimized_data(&packed_data, j_result.document_count)?;
    
    // Create empty metadata for now (could be enhanced to include parent metadata)
    let metadata = HashMap::new();
    
    Ok(EmbeddedExtractResult {
        documents,
        metadata,
    })
}

/// Unpack the optimized data format into EmbeddedDocument instances
fn unpack_optimized_data(data: &[u8], expected_count: i32) -> ExtractResult<Vec<EmbeddedDocument>> {
    let mut cursor = Cursor::new(data);
    let mut documents = Vec::with_capacity(expected_count as usize);
    
    // Read document count
    let count = read_i32(&mut cursor)?;
    if count != expected_count {
        return Err(Error::ParseError(format!(
            "Document count mismatch: expected {}, got {}",
            expected_count, count
        )));
    }
    
    // Read each document
    for _ in 0..count {
        let resource_name = read_string(&mut cursor)?;
        let content_type = read_string(&mut cursor)?;
        let embedded_relationship_id = read_string(&mut cursor)?;
        let embedded_relationship_id = if embedded_relationship_id.is_empty() {
            None
        } else {
            Some(embedded_relationship_id)
        };
        
        let content_len = read_i32(&mut cursor)?;
        let mut content = vec![0u8; content_len as usize];
        cursor.read_exact(&mut content).map_err(|e| {
            Error::ParseError(format!("Failed to read content: {}", e))
        })?;
        
        documents.push(EmbeddedDocument {
            resource_name,
            content_type,
            content,
            embedded_relationship_id,
        });
    }
    
    Ok(documents)
}

/// Read a 32-bit integer from the cursor (big-endian)
fn read_i32(cursor: &mut Cursor<&[u8]>) -> ExtractResult<i32> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf).map_err(|e| {
        Error::ParseError(format!("Failed to read i32: {}", e))
    })?;
    Ok(i32::from_be_bytes(buf))
}

/// Read a string from the cursor (length-prefixed UTF-8)
fn read_string(cursor: &mut Cursor<&[u8]>) -> ExtractResult<String> {
    let len = read_i32(cursor)?;
    let mut buf = vec![0u8; len as usize];
    cursor.read_exact(&mut buf).map_err(|e| {
        Error::ParseError(format!("Failed to read string data: {}", e))
    })?;
    String::from_utf8(buf).map_err(|e| {
        Error::ParseError(format!("Failed to decode string as UTF-8: {}", e))
    })
}