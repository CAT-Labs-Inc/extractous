use crate::errors::{Error, ExtractResult};
use crate::tika::jni_utils::{
    jni_call_method, jni_jobject_to_string, jni_new_string_as_jvalue,
    jni_tika_metadata_to_rust_metadata,
};
use crate::tika::vm;
use crate::{Metadata, OfficeParserConfig, PdfParserConfig, TesseractOcrConfig, DEFAULT_BUF_SIZE};
use bytemuck::cast_slice_mut;
use jni::objects::{GlobalRef, JByteArray, JObject, JString, JValue};
use jni::sys::jsize;
use jni::JNIEnv;

/// Wrapper for [`JObject`]s that contain `org.apache.commons.io.input.ReaderInputStream`
/// It saves a GlobalRef to the java object, which is cleared when the last GlobalRef is dropped
/// Implements [`Drop] trait to properly close the `org.apache.commons.io.input.ReaderInputStream`
#[derive(Clone)]
pub struct JReaderInputStream {
    /// Held in an `Option` purely so [`Drop`] can `take()` and release both
    /// global references *while the thread is still attached*. Struct fields are
    /// dropped after `Drop::drop` returns, by which point the `AttachGuard` has
    /// already detached — releasing a `GlobalRef` then makes the jni crate log
    /// "Dropping a GlobalRef in a detached thread" (twice per document).
    /// Always `Some` until `drop`; use [`Self::internal`] / [`Self::buffer`].
    internal: Option<GlobalRef>,
    buffer: Option<GlobalRef>,
    capacity: jsize,
}

impl JReaderInputStream {
    pub(crate) fn new<'local>(
        env: &mut JNIEnv<'local>,
        obj: JObject<'local>,
    ) -> ExtractResult<Self> {
        // Creates new jbyte array
        let capacity = DEFAULT_BUF_SIZE as jsize;
        let jbyte_array = env.new_byte_array(capacity)?;

        Ok(Self {
            internal: Some(env.new_global_ref(obj)?),
            buffer: Some(env.new_global_ref(jbyte_array)?),
            capacity,
        })
    }

    /// The wrapped `ReaderInputStream`. Only `None` after `drop` has run.
    fn internal(&self) -> &GlobalRef {
        self.internal
            .as_ref()
            .expect("JReaderInputStream used after drop")
    }

    /// The reusable Java read buffer. Only `None` after `drop` has run.
    fn buffer(&self) -> &GlobalRef {
        self.buffer
            .as_ref()
            .expect("JReaderInputStream used after drop")
    }

    pub(crate) fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut env = vm().attach_current_thread().map_err(Error::JniError)?;

        let length = buf.len() as jsize;

        if length > self.capacity {
            // Create the new byte array with the new capacity
            let jbyte_array = env
                .new_byte_array(length as jsize)
                .map_err(|_e| Error::JniEnvCall("Failed to create byte array"))?;

            self.buffer = Some(
                env.new_global_ref(jbyte_array)
                    .map_err(|_e| Error::JniEnvCall("Failed to create global reference"))?,
            );

            self.capacity = length;
        }

        // // Create the java byte array
        // let length = buf.len() as jsize;
        // let jbyte_array = env
        //     .new_byte_array(length)
        //     .map_err(|_e| Error::JniEnvCall("Failed to create byte array"))?;

        // cast because java byte array is i8[]
        let buf_of_i8: &mut [i8] = cast_slice_mut(buf);

        // `read` is called once per chunk for the whole document, so the local refs
        // it creates would otherwise accumulate for as long as the caller holds the
        // stream. The thread detach at the end of the call does eventually reclaim
        // them, but a frame popped per call bounds the local-reference table
        // regardless of how long a single extraction runs.
        let num_read_bytes = env
            .with_local_frame::<_, _, Error>(4, |env| {
                let call_result = jni_call_method(
                    env,
                    self.internal(),
                    "read",
                    "([BII)I",
                    &[
                        JValue::Object(self.buffer()),
                        JValue::Int(0),
                        JValue::Int(length),
                    ],
                );
                let num_read_bytes = call_result?.i().map_err(Error::JniError)?;

                // Get self.buffer object as a local reference
                let obj_local = env
                    .new_local_ref(self.buffer())
                    .map_err(|_e| Error::JniEnvCall("Failed to create local ref"))?;

                // Get the bytes from the Java byte array to the Rust byte array
                // This is a copy or just memory reference. POTENTIAL performance improvement
                env.get_byte_array_region(JByteArray::from(obj_local), 0, buf_of_i8)
                    .map_err(|_e| Error::JniEnvCall("Failed to get byte array region"))?;

                Ok(num_read_bytes)
            })
            .map_err(Error::from)?;

        if num_read_bytes == -1 {
            // End of stream reached
            Ok(0)
        } else {
            Ok(num_read_bytes as usize)
        }
    }
}

impl Drop for JReaderInputStream {
    fn drop(&mut self) {
        if let Ok(mut env) = vm().attach_current_thread() {
            // Close the Java reader first.
            if let Some(internal) = self.internal.as_ref() {
                jni_call_method(&mut env, internal, "close", "()V", &[]).ok();
            }
            // Then release both global refs *here*, while the thread is still
            // attached. Leaving them to the implicit field drop would run them
            // after this function returns — i.e. after the `AttachGuard` has
            // detached the thread — which is what produced the jni crate's
            // "Dropping a GlobalRef in a detached thread" warning twice per
            // document. If the attach itself failed there is nothing better to
            // do than fall back to the implicit drop.
            drop(self.internal.take());
            drop(self.buffer.take());
        }
    }
}

/// Wrapper for the Java class  `ai.yobix.StringResult`
/// Upon creation it parses the java StringResult object and saves the converted Rust string
pub struct JStringResult {
    pub content: String,
    pub metadata: Metadata,
}

impl<'local> JStringResult {
    pub(crate) fn new(env: &mut JNIEnv<'local>, obj: JObject<'local>) -> ExtractResult<Self> {
        let is_error = jni_call_method(env, &obj, "isError", "()Z", &[])?.z()?;

        if is_error {
            let status = jni_call_method(env, &obj, "getStatus", "()B", &[])?.b()?;
            let msg_obj = env
                .call_method(&obj, "getErrorMessage", "()Ljava/lang/String;", &[])?
                .l()?;
            let msg = jni_jobject_to_string(env, msg_obj)?;
            match status {
                1 => Err(Error::IoError(msg)),
                2 => Err(Error::ParseError(msg)),
                _ => Err(Error::Unknown(msg)),
            }
        } else {
            let call_result_obj = env
                .call_method(&obj, "getContent", "()Ljava/lang/String;", &[])?
                .l()?;
            let content = jni_jobject_to_string(env, call_result_obj)?;
            let tika_metadata_obj: JObject = env
                .call_method(
                    &obj,
                    "getMetadata",
                    "()Lorg/apache/tika/metadata/Metadata;",
                    &[],
                )?
                .l()?;
            let metadata = jni_tika_metadata_to_rust_metadata(env, tika_metadata_obj)?;
            Ok(Self { content, metadata })
        }
    }
}

/// Wrapper for the Java class  `ai.yobix.ReaderResult`
/// Upon creation it parses the java ReaderResult object and saves the java
/// `org.apache.commons.io.input.ReaderInputStream` object, which later can be used for reading
pub struct JReaderResult<'local> {
    pub java_reader: JObject<'local>,
    pub metadata: Metadata,
}

impl<'local> JReaderResult<'local> {
    pub(crate) fn new(env: &mut JNIEnv<'local>, obj: JObject<'local>) -> ExtractResult<Self> {
        let is_error = jni_call_method(env, &obj, "isError", "()Z", &[])?.z()?;

        if is_error {
            let status = jni_call_method(env, &obj, "getStatus", "()B", &[])?.b()?;
            let msg_obj = env
                .call_method(&obj, "getErrorMessage", "()Ljava/lang/String;", &[])?
                .l()?;
            let msg = jni_jobject_to_string(env, msg_obj)?;
            match status {
                1 => Err(Error::IoError(msg)),
                2 => Err(Error::ParseError(msg)),
                _ => Err(Error::Unknown(msg)),
            }
        } else {
            let reader_obj = jni_call_method(
                env,
                &obj,
                "getReader",
                "()Lorg/apache/commons/io/input/ReaderInputStream;",
                &[],
            )?
            .l()?;

            let tika_metadata_obj: JObject = env
                .call_method(
                    &obj,
                    "getMetadata",
                    "()Lorg/apache/tika/metadata/Metadata;",
                    &[],
                )?
                .l()?;
            let metadata = jni_tika_metadata_to_rust_metadata(env, tika_metadata_obj)?;

            Ok(Self {
                java_reader: reader_obj,
                metadata,
            })
        }
    }
}

/// Wrapper for [`JObject`]s that contain `org.apache.tika.parser.pdf.PDFParserConfig`.
/// Looks up the class and method IDs on creation rather than for every method call.
pub(crate) struct JPDFParserConfig<'local> {
    pub(crate) internal: JObject<'local>,
}

impl<'local> JPDFParserConfig<'local> {
    /// Creates a new object instance of `JPDFParserConfig` in the java world
    /// keeps reference to the object and method IDs for later use
    pub(crate) fn new(env: &mut JNIEnv<'local>, config: &PdfParserConfig) -> ExtractResult<Self> {
        // Create the java object
        let class = env.find_class("org/apache/tika/parser/pdf/PDFParserConfig")?;
        let obj = env.new_object(&class, "()V", &[])?;

        // Call the setters
        // Make sure all of these methods are declared in jni-config.json file, otherwise
        // java method not found exception will be thrown
        jni_call_method(
            env,
            &obj,
            "setExtractInlineImages",
            "(Z)V",
            &[JValue::from(config.extract_inline_images)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setExtractUniqueInlineImagesOnly",
            "(Z)V",
            &[JValue::from(config.extract_unique_inline_images_only)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setExtractMarkedContent",
            "(Z)V",
            &[JValue::from(config.extract_marked_content)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setExtractAnnotationText",
            "(Z)V",
            &[JValue::from(config.extract_annotation_text)],
        )?;
        // The PdfOcrStrategy enum names must match the Java org.apache.tika.parser.pdf
        // .PDFParserConfig$OCR_STRATEGY enum names
        let ocr_str_val = jni_new_string_as_jvalue(env, &config.ocr_strategy.to_string())?;
        jni_call_method(
            env,
            &obj,
            "setOcrStrategy",
            "(Ljava/lang/String;)V",
            &[(&ocr_str_val).into()],
        )?;

        Ok(Self { internal: obj })
    }
}

/// Wrapper for [`JObject`]s that contain `org.apache.tika.parser.microsoft.OfficeParserConfig`.
pub(crate) struct JOfficeParserConfig<'local> {
    pub(crate) internal: JObject<'local>,
}

impl<'local> JOfficeParserConfig<'local> {
    /// Creates a new object instance of `JOfficeParserConfig` in the java world
    /// keeps reference to the object for later use
    pub(crate) fn new(
        env: &mut JNIEnv<'local>,
        config: &OfficeParserConfig,
    ) -> ExtractResult<Self> {
        // Create the java object
        let class = env.find_class("org/apache/tika/parser/microsoft/OfficeParserConfig")?;
        let obj = env.new_object(&class, "()V", &[])?;

        // Call the setters
        // Make sure all of these methods are declared in jni-config.json file, otherwise
        // java method not found exception will be thrown
        jni_call_method(
            env,
            &obj,
            "setExtractMacros",
            "(Z)V",
            &[JValue::from(config.extract_macros)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setIncludeDeletedContent",
            "(Z)V",
            &[JValue::from(config.include_deleted_content)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setIncludeMoveFromContent",
            "(Z)V",
            &[JValue::from(config.include_move_from_content)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setIncludeShapeBasedContent",
            "(Z)V",
            &[JValue::from(config.include_shape_based_content)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setIncludeHeadersAndFooters",
            "(Z)V",
            &[JValue::from(config.include_headers_and_footers)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setIncludeMissingRows",
            "(Z)V",
            &[JValue::from(config.include_missing_rows)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setIncludeSlideNotes",
            "(Z)V",
            &[JValue::from(config.include_slide_notes)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setIncludeSlideMasterContent",
            "(Z)V",
            &[JValue::from(config.include_slide_master_content)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setConcatenatePhoneticRuns",
            "(Z)V",
            &[JValue::from(config.concatenate_phonetic_runs)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setExtractAllAlternativesFromMSG",
            "(Z)V",
            &[JValue::from(config.extract_all_alternatives_from_msg)],
        )?;

        Ok(Self { internal: obj })
    }
}

/// Wrapper for [`JObject`]s that contain `org.apache.tika.parser.ocr.TesseractOCRConfig`.
pub(crate) struct JTesseractOcrConfig<'local> {
    pub(crate) internal: JObject<'local>,
}
impl<'local> JTesseractOcrConfig<'local> {
    /// Creates a new object instance of `JTesseractOcrConfig` in the java world
    /// keeps reference to the object for later use
    pub(crate) fn new(
        env: &mut JNIEnv<'local>,
        config: &TesseractOcrConfig,
    ) -> ExtractResult<Self> {
        // Create the java object
        let class = env.find_class("org/apache/tika/parser/ocr/TesseractOCRConfig")?;
        let obj = env.new_object(&class, "()V", &[])?;

        // Call the setters
        // Make sure all of these methods are declared in jni-config.json file, otherwise
        // java method not found exception will be thrown
        jni_call_method(
            env,
            &obj,
            "setDensity",
            "(I)V",
            &[JValue::from(config.density)],
        )?;
        jni_call_method(env, &obj, "setDepth", "(I)V", &[JValue::from(config.depth)])?;
        jni_call_method(
            env,
            &obj,
            "setTimeoutSeconds",
            "(I)V",
            &[JValue::from(config.timeout_seconds)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setEnableImagePreprocessing",
            "(Z)V",
            &[JValue::from(config.enable_image_preprocessing)],
        )?;
        jni_call_method(
            env,
            &obj,
            "setApplyRotation",
            "(Z)V",
            &[JValue::from(config.apply_rotation)],
        )?;

        let lang_string_val = jni_new_string_as_jvalue(env, &config.language)?;
        jni_call_method(
            env,
            &obj,
            "setLanguage",
            "(Ljava/lang/String;)V",
            &[(&lang_string_val).into()],
        )?;

        Ok(Self { internal: obj })
    }
}

/// Wrapper for [`JObject`]s that contain `ai.yobix.EmbeddedExtractResult`.
pub(crate) struct JEmbeddedExtractResult {
    pub(crate) error_code: u8,
    pub(crate) error_message: Option<String>,
    pub(crate) documents: Vec<JEmbeddedDocument>,
    pub(crate) metadata: Metadata,
}

impl JEmbeddedExtractResult {
    pub(crate) fn new<'local>(
        env: &mut JNIEnv<'local>,
        obj: JObject<'local>,
    ) -> ExtractResult<Self> {
        // Get error code
        let error_code = jni_call_method(env, &obj, "getErrorCode", "()B", &[])?
            .b()
            .unwrap_or(0) as u8;

        // Get error message if error
        let error_message = if error_code != 0 {
            let msg_obj = jni_call_method(env, &obj, "getErrorMessage", "()Ljava/lang/String;", &[])?
                .l()?;
            if !msg_obj.is_null() {
                Some(jni_jobject_to_string(env, msg_obj)?)
            } else {
                None
            }
        } else {
            None
        };

        // Get embedded documents list
        let docs_list = jni_call_method(
            env,
            &obj,
            "getEmbeddedDocuments",
            "()Ljava/util/List;",
            &[],
        )?
        .l()?;

        // Convert Java List to Vec
        let size = jni_call_method(env, &docs_list, "size", "()I", &[])?.i()? as usize;
        let mut documents = Vec::with_capacity(size);

        for i in 0..size {
            // Each iteration creates ~5 JNI local references (the list element plus
            // the resource name, content type, content array and relationship id
            // fetched inside `JEmbeddedDocument::new`). With up to
            // MAX_EMBEDDED_ITEMS_PER_FILE (1000) documents that was ~5000 refs held
            // live at once, keeping every Java byte[] reachable — and so
            // unreclaimable by the collector — for the whole conversion. The
            // per-call thread detach used to clean these up; threads now stay
            // attached, so each ref is released explicitly instead, at the end of
            // the iteration that made it. `JEmbeddedDocument` copies out to owned
            // Rust data, so nothing needs the refs afterwards.
            let doc_obj = jni_call_method(
                env,
                &docs_list,
                "get",
                "(I)Ljava/lang/Object;",
                &[JValue::from(i as i32)],
            )?
            .l()?;
            let doc_obj = env.auto_local(doc_obj);
            documents.push(JEmbeddedDocument::new(env, &doc_obj)?);
        }

        // For now, we'll create empty metadata
        // TODO: Get metadata from parent document if needed
        let metadata = Metadata::new();

        Ok(Self {
            error_code,
            error_message,
            documents,
            metadata,
        })
    }
}

/// Wrapper for `ai.yobix.EmbeddedExtractResult$EmbeddedDocument`
pub(crate) struct JEmbeddedDocument {
    pub(crate) resource_name: String,
    pub(crate) content_type: String,
    pub(crate) content: Vec<u8>,
    pub(crate) embedded_relationship_id: Option<String>,
}

impl JEmbeddedDocument {
    /// Copies one embedded document out to owned Rust data.
    ///
    /// Every local reference created here is wrapped in an `AutoLocal` so it is
    /// deleted when this function returns. Threads stay attached to the isolate
    /// now (see `get_vm_attach_current_thread`), so nothing else would release
    /// them — and holding the content `byte[]` refs live across the whole
    /// document list is what previously pinned every extracted image in the Java
    /// heap simultaneously.
    pub(crate) fn new<'local>(
        env: &mut JNIEnv<'local>,
        obj: &JObject<'local>,
    ) -> ExtractResult<Self> {
        // Get resource name
        let name_obj = jni_call_method(env, obj, "getResourceName", "()Ljava/lang/String;", &[])?
            .l()?;
        let resource_name = Self::take_string(env, name_obj)?.unwrap_or_default();

        // Get content type
        let type_obj = jni_call_method(env, obj, "getContentType", "()Ljava/lang/String;", &[])?
            .l()?;
        let content_type = Self::take_string(env, type_obj)?.unwrap_or_default();

        // Get content bytes
        let content_array = jni_call_method(env, obj, "getContent", "()[B", &[])?.l()?;
        let was_null = content_array.is_null();
        let array = env.auto_local(JByteArray::from(content_array));
        let content = if was_null {
            Vec::new()
        } else {
            let len = env.get_array_length(&*array)?;
            let mut content = vec![0u8; len as usize];
            env.get_byte_array_region(&*array, 0, cast_slice_mut(&mut content))?;
            content
        };

        // Get embedded relationship id
        let rel_obj = jni_call_method(
            env,
            obj,
            "getEmbeddedRelationshipId",
            "()Ljava/lang/String;",
            &[],
        )?
        .l()?;
        let embedded_relationship_id = Self::take_string(env, rel_obj)?;

        Ok(Self {
            resource_name,
            content_type,
            content,
            embedded_relationship_id,
        })
    }

    /// Reads a Java `String` local ref into an owned Rust `String` and releases
    /// the reference, returning `None` for a Java null.
    ///
    /// `jni_jobject_to_string` takes the `JObject` by value and leaves the local
    /// reference alive, which is only safe when a thread detach later reclaims
    /// it. Threads now stay attached, so this releases the ref explicitly.
    fn take_string<'local>(
        env: &mut JNIEnv<'local>,
        obj: JObject<'local>,
    ) -> ExtractResult<Option<String>> {
        if obj.is_null() {
            return Ok(None);
        }
        let jstr = env.auto_local(JString::from(obj));
        let value = env.get_string(&jstr)?.to_string_lossy().into_owned();
        Ok(Some(value))
    }
}

/// Wrapper for `ai.yobix.OptimizedEmbeddedExtractor$OptimizedResult`
pub(crate) struct JOptimizedResult {
    pub(crate) error_code: u8,
    pub(crate) error_message: Option<String>,
    pub(crate) packed_data: Option<Vec<u8>>,
    pub(crate) document_count: i32,
}

impl JOptimizedResult {
    pub(crate) fn new<'local>(
        env: &mut JNIEnv<'local>,
        obj: JObject<'local>,
    ) -> ExtractResult<Self> {
        // Get error code
        let error_code = jni_call_method(env, &obj, "getErrorCode", "()B", &[])?
            .b()
            .unwrap_or(0) as u8;

        // Get error message if error
        let error_message = if error_code != 0 {
            let msg_obj = jni_call_method(env, &obj, "getErrorMessage", "()Ljava/lang/String;", &[])?
                .l()?;
            if !msg_obj.is_null() {
                Some(jni_jobject_to_string(env, msg_obj)?)
            } else {
                None
            }
        } else {
            None
        };

        // Get document count
        let document_count = jni_call_method(env, &obj, "getDocumentCount", "()I", &[])?.i()?;

        // Get packed data buffer if success
        let packed_data = if error_code == 0 {
            let buffer_obj = jni_call_method(
                env,
                &obj,
                "getPackedData",
                "()Ljava/nio/ByteBuffer;",
                &[],
            )?
            .l()?;

            if !buffer_obj.is_null() {
                // Get buffer capacity
                let capacity = jni_call_method(env, &buffer_obj, "capacity", "()I", &[])?.i()?;
                
                // Rewind buffer to start
                jni_call_method(env, &buffer_obj, "rewind", "()Ljava/nio/Buffer;", &[])?;

                // Create byte array and bulk get
                let array = env.new_byte_array(capacity)?;
                let array_obj = JObject::from(array);
                jni_call_method(
                    env,
                    &buffer_obj,
                    "get",
                    "([B)Ljava/nio/ByteBuffer;",
                    &[JValue::Object(&array_obj)],
                )?;

                // Convert to Vec<u8>
                let mut data = vec![0u8; capacity as usize];
                env.get_byte_array_region(&JByteArray::from(array_obj), 0, cast_slice_mut(&mut data))?;
                Some(data)
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            error_code,
            error_message,
            packed_data,
            document_count,
        })
    }
}
