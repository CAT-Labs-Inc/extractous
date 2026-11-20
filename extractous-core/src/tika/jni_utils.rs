use std::env;
use std::ffi::CString;
use std::os::raw::{c_char, c_void};

use crate::errors::{Error, ExtractResult};
use crate::Metadata;
use jni::errors::jni_error_code_to_result;
use jni::objects::{JByteBuffer, JObject, JObjectArray, JString, JValue, JValueOwned};
use jni::{sys, JNIEnv, JavaVM};
use std::collections::HashMap;

/// Calls a static method and prints any thrown exceptions to stderr
pub fn jni_new_direct_buffer<'local>(
    env: &mut JNIEnv<'local>,
    data: *mut u8,
    len: usize,
) -> ExtractResult<JByteBuffer<'local>> {
    let direct_byte_buffer = unsafe { env.new_direct_byte_buffer(data, len) }
        .map_err(|_e| Error::JniEnvCall("Failed to create direct byte buffer"))?;

    Ok(direct_byte_buffer)
}

/// Calls a static method and prints any thrown exceptions to stderr
pub fn jni_call_static_method<'local>(
    env: &mut JNIEnv<'local>,
    class: &str,
    method: &str,
    signature: &str,
    args: &[JValue],
) -> ExtractResult<JValueOwned<'local>> {
    let call_result = env.call_static_method(class, method, signature, args);
    match call_result {
        Ok(result) => Ok(result),
        Err(error) => match error {
            jni::errors::Error::JavaException => {
                jni_check_exception(env)?;
                Err(Error::JniError(error))
            }
            _ => Err(Error::JniError(error)),
        },
    }
}

/// Calls an object method and prints any thrown exceptions to stderr
pub fn jni_call_method<'local>(
    env: &mut JNIEnv<'local>,
    obj: &JObject<'local>,
    method: &str,
    signature: &str,
    args: &[JValue],
) -> ExtractResult<JValueOwned<'local>> {
    let call_result = env.call_method(obj, method, signature, args);
    match call_result {
        Ok(result) => Ok(result),
        Err(error) => match error {
            jni::errors::Error::JavaException => {
                jni_check_exception(env)?;
                Err(Error::JniError(error))
            }
            _ => Err(Error::JniError(error)),
        },
    }
}

/// creates a new java string from a rust str
pub fn jni_new_string<'local>(env: &mut JNIEnv<'local>, s: &str) -> ExtractResult<JString<'local>> {
    match env.new_string(s) {
        Ok(s) => Ok(s),
        Err(_) => Err(Error::JniEnvCall("Couldn't create Java String")),
    }
}

/// creates a new java string from a rust str and returns it as a JValueOwned
pub fn jni_new_string_as_jvalue<'local>(
    env: &mut JNIEnv<'local>,
    s: &str,
) -> ExtractResult<JValueOwned<'local>> {
    let jstring = jni_new_string(env, s)?;
    //let jstring = env.new_string(s)?;

    Ok(JValueOwned::from(jstring))
}

/// Converts a java object to a rust string
pub fn jni_jobject_to_string<'local>(
    env: &mut JNIEnv<'local>,
    jobject: JObject<'local>,
) -> ExtractResult<String> {
    let jstring_output = JString::from(jobject);
    let javastr_output = unsafe { env.get_string_unchecked(&jstring_output)? };
    let output_str = javastr_output.to_string_lossy();
    //let output_str = javastr_output.to_str().map_err(Error::Utf8Error)?;

    Ok(output_str.to_string())
}

/// Converts a Java String[] to a Rust Vec<String>
pub fn jni_jobject_array_to_vec<'local>(
    env: &mut JNIEnv<'local>,
    array: JObject<'local>,
) -> ExtractResult<Vec<String>> {
    let j_array_string = JObjectArray::from(array);
    let j_array_length = env.get_array_length(&j_array_string)?;

    let mut vec = Vec::with_capacity(j_array_length as usize);

    for i in 0..j_array_length {
        let elem_obj = env.get_object_array_element(&j_array_string, i)?;
        let elem_str = jni_jobject_to_string(env, elem_obj)?;
        vec.push(elem_str);
    }

    Ok(vec)
}

/// Convert a Tika Metadata a Rust Metadata
pub fn jni_tika_metadata_to_rust_metadata<'local>(
    env: &mut JNIEnv<'local>,
    j_tika_metadata_object: JObject<'local>,
) -> ExtractResult<Metadata> {
    let j_keys_names = env
        .call_method(
            &j_tika_metadata_object,
            "names",
            "()[Ljava/lang/String;",
            &[],
        )?
        .l()?;
    let keys_names = jni_jobject_array_to_vec(env, j_keys_names)?;
    let mut metadata = HashMap::new();
    for key_name in keys_names.iter() {
        let j_key_name = jni_new_string_as_jvalue(env, key_name)?;
        let j_obj_array_name_metadata = env
            .call_method(
                &j_tika_metadata_object,
                "getValues",
                "(Ljava/lang/String;)[Ljava/lang/String;",
                &[(&j_key_name).into()],
            )?
            .l()?;
        let key_metadata = jni_jobject_array_to_vec(env, j_obj_array_name_metadata)?;
        metadata.insert(key_name.to_string(), key_metadata);
    }
    Ok(metadata)
}

/// Checks if there is an exception in the jni environment, describes it to
/// the stderr and finally clears it
pub fn jni_check_exception(env: &mut JNIEnv) -> ExtractResult<bool> {
    if env.exception_check()? {
        env.exception_describe()?;
        env.exception_clear()?;
        return Ok(true);
    }
    Ok(false)
}

/// Reads an isolate sizing option from the environment, returning the full VM
/// option string (e.g. `-Xmx4g`) when the variable is set and non-empty.
///
/// GraalVM CE native images only ship the Serial collector (G1 is Oracle
/// GraalVM + Linux only), and this isolate is shared by every Rayon worker, all
/// allocating extracted image buffers concurrently. Serial GC is stop-the-world
/// and single-threaded, so collection frequency — governed largely by young-gen
/// size — bounds how well extraction scales across cores.
///
/// No defaults are applied: unset variables leave the isolate exactly as it was
/// configured before, so this cannot regress an existing deployment. Tune with
/// `EXTRACTOUS_MAX_HEAP` (`-Xmx`) and `EXTRACTOUS_YOUNG_GEN` (`-Xmn`).
fn sizing_option(var: &str, flag: &str) -> Option<CString> {
    let value = env::var(var).ok()?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    CString::new(format!("{flag}{value}")).ok()
}

/// Creates a new graalvm isolate using the invocation api. A [GraalVM isolate](https://medium.com/graalvm/isolates-and-compressed-references-more-flexible-and-efficient-memory-management-for-graalvm-a044cc50b67e) is a disjoint heap
/// that allows multiple tasks in the same VM instance to run independently.
///
/// This function uses the standard JVM invocation API and relies on the jni-sys crate.
/// No need to specify any libraries because the graalvm native image is already
/// linked in by the build script.
pub fn create_vm_isolate() -> JavaVM {
    unsafe {
        // These strings are handed to a C API and must stay alive, and
        // NUL-terminated, until after JNI_CreateJavaVM returns — so own them in a
        // vec that outlives the call rather than building them inline.
        let mut option_strings: Vec<CString> = vec![
            // Set java.library.path to be able to load libawt.so, which must be in the same dir as libtika_native.so
            CString::new("-Djava.library.path=.").expect("static option"),
            // enable awt headless mode
            // NOTE: this MUST begin with "-D". Without it the option is malformed and,
            // because the VM is created with ignoreUnrecognized=JNI_TRUE, it is silently
            // dropped — leaving AWT non-headless, which deadlocks when PDFBox decodes
            // (inline) images inside this embedded native image (no window server).
            CString::new("-Djava.awt.headless=true").expect("static option"),
        ];
        option_strings.extend(sizing_option("EXTRACTOUS_MAX_HEAP", "-Xmx"));
        option_strings.extend(sizing_option("EXTRACTOUS_YOUNG_GEN", "-Xmn"));

        let vm_options: Vec<sys::JavaVMOption> = option_strings
            .iter()
            .map(|opt| sys::JavaVMOption {
                optionString: opt.as_ptr() as *mut c_char,
                extraInfo: std::ptr::null_mut(),
            })
            .collect();

        // Malformed options are silently dropped under JNI_TRUE, which has already
        // cost us once (see the headless note above). Set
        // EXTRACTOUS_STRICT_VM_OPTIONS=1 to make the VM refuse to start on an
        // unrecognized option, so tuning can be validated rather than assumed.
        let strict = env::var("EXTRACTOUS_STRICT_VM_OPTIONS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let mut args = sys::JavaVMInitArgs {
            version: sys::JNI_VERSION_1_8,
            nOptions: vm_options.len() as sys::jint,
            options: vm_options.as_ptr() as *mut sys::JavaVMOption,
            ignoreUnrecognized: if strict {
                sys::JNI_FALSE
            } else {
                sys::JNI_TRUE
            },
        };
        let mut ptr: *mut sys::JavaVM = std::ptr::null_mut();
        let mut env: *mut sys::JNIEnv = std::ptr::null_mut();

        // The current thread becomes the main thread
        let jni_res = sys::JNI_CreateJavaVM(
            &mut ptr as *mut _,
            &mut env as *mut *mut sys::JNIEnv as *mut *mut c_void,
            &mut args as *mut sys::JavaVMInitArgs as *mut c_void,
        );
        jni_error_code_to_result(jni_res).unwrap_or_else(|e| {
            panic!("Failed creating the graal native vm: {:?}", e);
        });

        // This sys call already attaches the current thread to the vm
        JavaVM::from_raw(ptr).unwrap_or_else(|e| {
            panic!("Failed creating the graal native from pointer: {:?}", e);
        })
    }
}

// fn cleanup_vm_isolate(jvm: JavaVM) -> ExtractResult<()>  {
//     println!("cleanup_vm_isolate");
//     // let mut env = jvm.attach_current_thread_as_daemon()?;
//     //
//     // let x = JValue::from(1);
//     // let system_class = env.find_class("java/lang/System")?;
//     // let exit_mid = env.get_static_method_id(&system_class, "exit", "(I)V")?;
//     // let _val = unsafe {
//     //     env.call_static_method_unchecked(
//     //         &system_class,
//     //         exit_mid,
//     //         ReturnType::Primitive(Primitive::Void),
//     //         &[x.as_jni()],
//     //     )
//     // };
//
//     // Destroy jvm. jvm must be dropped as well
//     unsafe {  jvm.destroy()?; }
//     drop(jvm);
//
//     Ok(())
// }

// pub fn tika_parse_file_new_vm(file_name: &str) -> ExtractResult<String> {
//
//     let mut output = String::new();
//
//     let mut start_time = Instant::now();
//     let jvm = create_vm_isolate();
//     let jvm_create_duration = start_time.elapsed();
//
//     start_time = Instant::now();
//     // Need to create a new scope to be able to drop intermediate objects before destroying the jvm
//     {
//         //let mut env = jvm.get_env()?;
//         let mut env = jvm.attach_current_thread()?;
//
//         let jstr_file = env.new_string(file_name)?;
//         let val = env.call_static_method("ai/yobix/TikaNativeMain", "parseToString",
//                                          "(Ljava/lang/String;)Ljava/lang/String;", &[JValue::from(&jstr_file)])?;
//
//         let jobject = val.l()?;
//         let jstr_output = JString::from(jobject);
//         let javastr_output = env.get_string(&jstr_output)?;
//         let output_str = javastr_output.to_str().map_err(|e| Error::Utf8Error(e))?;
//         // Creates the string before cleaning the vm
//         output.push_str(output_str);
//     }
//     let parse_duration = start_time.elapsed();
//
//     start_time = Instant::now();
//     cleanup_vm_isolate(jvm)?;
//     let jvm_destroy_duration = start_time.elapsed();
//
//     println!("Time taken to jvm_create_duration: {:.4?}", jvm_create_duration);
//     println!("Time taken to parse_duration: {:.4?}", parse_duration);
//     println!("Time taken to jvm_destroy_duration: {:.4?}", jvm_destroy_duration);
//
//     Ok(output)
// }
