use crate::embedded::{EmbeddedDocument, EmbeddedExtractResult};
use crate::errors::{Error, ExtractResult};
use crate::tika::jni_utils::{jni_call_static_method, jni_new_string_as_jvalue};
use crate::tika::vm;
use crate::tika::wrappers::{JEmbeddedExtractResult, JOfficeParserConfig, JPDFParserConfig, JTesseractOcrConfig};
use crate::{OfficeParserConfig, PdfParserConfig, TesseractOcrConfig};
use jni::objects::{JValue};
use jni::AttachGuard;

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

/// Extract embedded documents from a file
pub fn extract_embedded_from_file(
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
    
    // Call the Java method
    let result = jni_call_static_method(
        &mut env,
        "ai/yobix/TikaNativeMain",
        "extractEmbedded",
        "(Ljava/lang/String;\
        Lorg/apache/tika/parser/pdf/PDFParserConfig;\
        Lorg/apache/tika/parser/microsoft/OfficeParserConfig;\
        Lorg/apache/tika/parser/ocr/TesseractOCRConfig;\
        )Lai/yobix/EmbeddedExtractResult;",
        &[
            (&file_path_val).into(),
            (&j_pdf_conf.internal).into(),
            (&j_office_conf.internal).into(),
            (&j_ocr_conf.internal).into(),
        ],
    )?;
    
    let result_obj = result.l()?;
    
    // Convert Java result to Rust
    let j_result = JEmbeddedExtractResult::new(&mut env, result_obj)?;
    
    // Check for errors
    if j_result.error_code != 0 {
        let error_msg = j_result.error_message.unwrap_or_else(|| {
            format!("Embedded extraction failed with code {}", j_result.error_code)
        });
        return Err(Error::ParseError(error_msg));
    }
    
    // Convert Java documents to Rust documents
    let mut documents = Vec::with_capacity(j_result.documents.len());
    for j_doc in j_result.documents {
        documents.push(EmbeddedDocument {
            resource_name: j_doc.resource_name,
            content_type: j_doc.content_type,
            content: j_doc.content,
            embedded_relationship_id: j_doc.embedded_relationship_id,
        });
    }
    
    // Use the metadata from j_result
    let metadata = j_result.metadata;
    
    Ok(EmbeddedExtractResult {
        documents,
        metadata,
    })
}

/// Extract embedded documents from bytes
pub fn extract_embedded_from_bytes(
    buffer: &[u8],
    pdf_conf: &PdfParserConfig,
    office_conf: &OfficeParserConfig,
    ocr_conf: &TesseractOcrConfig,
) -> ExtractResult<EmbeddedExtractResult> {
    let mut env = get_vm_attach_current_thread()?;
    
    // Create ByteBuffer from the byte array
    // Note: new_direct_byte_buffer requires a mutable pointer, so we need to copy the data
    let mut buffer_copy = buffer.to_vec();
    let byte_buffer = unsafe {
        env.new_direct_byte_buffer(buffer_copy.as_mut_ptr(), buffer_copy.len())?
    };
    
    // Create Java config objects
    let j_pdf_conf = JPDFParserConfig::new(&mut env, pdf_conf)?;
    let j_office_conf = JOfficeParserConfig::new(&mut env, office_conf)?;
    let j_ocr_conf = JTesseractOcrConfig::new(&mut env, ocr_conf)?;
    
    // Call the Java method
    let result = jni_call_static_method(
        &mut env,
        "ai/yobix/TikaNativeMain",
        "extractEmbeddedFromBytes",
        "(Ljava/nio/ByteBuffer;\
        Lorg/apache/tika/parser/pdf/PDFParserConfig;\
        Lorg/apache/tika/parser/microsoft/OfficeParserConfig;\
        Lorg/apache/tika/parser/ocr/TesseractOCRConfig;\
        )Lai/yobix/EmbeddedExtractResult;",
        &[
            JValue::Object(&byte_buffer),
            (&j_pdf_conf.internal).into(),
            (&j_office_conf.internal).into(),
            (&j_ocr_conf.internal).into(),
        ],
    )?;
    
    let result_obj = result.l()?;
    
    // Convert Java result to Rust
    let j_result = JEmbeddedExtractResult::new(&mut env, result_obj)?;
    
    // Check for errors
    if j_result.error_code != 0 {
        let error_msg = j_result.error_message.unwrap_or_else(|| {
            format!("Embedded extraction from bytes failed with code {}", j_result.error_code)
        });
        return Err(Error::ParseError(error_msg));
    }
    
    // Convert Java documents to Rust documents
    let mut documents = Vec::with_capacity(j_result.documents.len());
    for j_doc in j_result.documents {
        documents.push(EmbeddedDocument {
            resource_name: j_doc.resource_name,
            content_type: j_doc.content_type,
            content: j_doc.content,
            embedded_relationship_id: j_doc.embedded_relationship_id,
        });
    }
    
    // Use the metadata from j_result
    let metadata = j_result.metadata;
    
    Ok(EmbeddedExtractResult {
        documents,
        metadata,
    })
}