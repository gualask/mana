//! The TV's language, as a locale code. On webOS it is the UI locale the
//! settings service gives over luna (`settings.localeInfo.locales.UI`), read
//! once on its own thread, as plx-native and webos-native-app-kit read it:
//! LG's SDL is 2.0.10, without `SDL_GetPreferredLocales` (2.0.14). The call
//! goes through `HLunaServiceCall` of `libhelpers.so.2`, as
//! webos-native-app-kit and moonlight-tv make it, loaded with dlopen: a
//! firmware without it gives English rather than a failed start. On the
//! desktop build it is `LC_ALL`, `LC_MESSAGES` or `LANG`.

use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

/// How long the TV's answer is waited for, as plx-native and
/// webos-native-app-kit wait for it at start.
pub const LIMIT: Duration = Duration::from_millis(600);

/// A read of the TV's locale code in progress.
pub struct Reading {
    answer: Receiver<Result<String, String>>,
    deadline: Instant,
}

impl Reading {
    /// Starts reading the TV's locale code.
    pub fn start() -> Self {
        let (sender, answer) = sync_channel(1);
        let deadline = Instant::now() + LIMIT;
        // A thread that does not start drops the sender: poll says so.
        #[cfg(webos)]
        drop(std::thread::Builder::new().name("locale".into()).spawn(move || drop(sender.send(read()))));
        #[cfg(not(webos))]
        drop(sender.send(mana_core::language::Language::environment_locale()));
        Self { answer, deadline }
    }

    /// The code, or why there is none, once known; None while waiting. Past
    /// LIMIT the read has failed, whatever comes later.
    pub fn poll(&self) -> Option<Result<String, String>> {
        match self.answer.try_recv() {
            Ok(answer) => Some(answer),
            Err(TryRecvError::Disconnected) => Some(Err("the read did not start".into())),
            Err(TryRecvError::Empty) if Instant::now() >= self.deadline => {
                Some(Err(format!("no answer within {} ms", LIMIT.as_millis())))
            }
            Err(TryRecvError::Empty) => None,
        }
    }
}

/// The UI locale in a reply of `getSystemSettings` for `localeInfo`.
#[cfg(any(webos, test))]
fn locale_of_reply(reply: &str) -> Result<String, String> {
    let json: serde_json::Value = serde_json::from_str(reply).map_err(|e| format!("reply not JSON: {e}"))?;
    if json.get("returnValue") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("reply without returnValue true: {}", shortened(reply)));
    }
    json.pointer("/settings/localeInfo/locales/UI")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("reply without locales.UI: {}", shortened(reply)))
}

/// The start of `reply`, for the log.
#[cfg(any(webos, test))]
fn shortened(reply: &str) -> String {
    reply.chars().take(120).collect()
}


/// Asks the settings service and waits for its reply; `Reading` gives up
/// at LIMIT, and a reply that never comes leaves this thread waiting.
#[cfg(webos)]
fn read() -> Result<String, String> {
    let reply = luna::call("luna://com.webos.settingsservice/getSystemSettings", r#"{"keys":["localeInfo"]}"#)?;
    locale_of_reply(&reply)
}

#[cfg(webos)]
mod luna {
    use std::ffi::{CStr, CString, c_char, c_int, c_ulong, c_void};
    use std::sync::mpsc::{SyncSender, sync_channel};

    use libloading::Library;

    const LIBRARY: &str = "libhelpers.so.2";

    type Callback = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> bool;
    type CallFn = unsafe extern "C" fn(*const c_char, *const c_char, *mut HContext) -> c_int;
    type MessageFn = unsafe extern "C" fn(*mut c_void) -> *const c_char;

    /// libhelpers.h's `HContext`, as webos-native-app-kit reproduces it.
    #[repr(C)]
    struct HContext {
        callback: Callback,
        userdata: *mut c_void,
        unknown: *mut c_void,
        multiple: c_int,
        public: c_int,
        ret_token: c_ulong,
    }

    /// One call: libhelpers hands the context back as the callback's last
    /// argument, so the context comes first, with room after it for fields
    /// libhelpers may keep past the public ones.
    #[repr(C)]
    struct Call {
        context: HContext,
        _private: [u8; 128],
        message: MessageFn,
        reply: SyncSender<String>,
    }

    unsafe extern "C" fn on_reply(_handle: *mut c_void, message: *mut c_void, context: *mut c_void) -> bool {
        // SAFETY: `context` is the leaked Call's, which never goes away.
        let call = unsafe { &*context.cast::<Call>() };
        // SAFETY: the message is valid during the callback; its text is
        // NUL-terminated.
        let text = unsafe { (call.message)(message) };
        let text = if text.is_null() { String::new() } else { unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned() };
        drop(call.reply.try_send(text));
        true
    }

    /// The reply to `payload` at `uri` on the public bus, once it comes.
    pub(super) fn call(uri: &str, payload: &str) -> Result<String, String> {
        // SAFETY: a firmware library without load-time preconditions; it
        // stays loaded, since a late reply may still call back.
        let library = unsafe { Library::new(LIBRARY) }.map_err(|e| format!("{LIBRARY}: {e}"))?;
        // SAFETY: the signatures of libhelpers.h.
        let (call_fn, message): (CallFn, MessageFn) = unsafe {
            let call_fn = *library.get::<CallFn>(b"HLunaServiceCall\0").map_err(|e| format!("HLunaServiceCall: {e}"))?;
            let message = *library.get::<MessageFn>(b"HLunaServiceMessage\0").map_err(|e| format!("HLunaServiceMessage: {e}"))?;
            (call_fn, message)
        };
        std::mem::forget(library);
        let (reply, replies) = sync_channel(1);
        let context = HContext {
            callback: on_reply,
            userdata: std::ptr::null_mut(),
            unknown: std::ptr::null_mut(),
            multiple: 0,
            public: 1,
            ret_token: 0,
        };
        // Never freed: a reply after the limit must still find it.
        let call = Box::leak(Box::new(Call { context, _private: [0; 128], message, reply }));
        let (uri, payload) = (CString::new(uri).map_err(|e| e.to_string())?, CString::new(payload).map_err(|e| e.to_string())?);
        // SAFETY: NUL-terminated strings that outlive the call; the context
        // outlives any reply.
        if unsafe { call_fn(uri.as_ptr(), payload.as_ptr(), &mut call.context) } != 0 {
            return Err("HLunaServiceCall failed".into());
        }
        // The sender lives in the leaked call: this waits for the reply.
        replies.recv().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ui_locale_comes_from_a_successful_reply_only() {
        let reply = r#"{"method":"getSystemSettings","returnValue":true,"settings":{"localeInfo":{"locales":{"UI":"it-IT","TV":"it-IT","FMT":"it-IT"},"clock":"locale"}}}"#;
        assert_eq!(locale_of_reply(reply), Ok("it-IT".into()));
        let refused = r#"{"returnValue":false,"errorCode":-1,"errorText":"denied"}"#;
        assert!(locale_of_reply(refused).unwrap_err().contains("returnValue"));
        let without = r#"{"returnValue":true,"settings":{"localeInfo":{"locales":{"TV":"de-DE"}}}}"#;
        assert!(locale_of_reply(without).unwrap_err().contains("locales.UI"));
        assert!(locale_of_reply("{\"returnValue\":tr").unwrap_err().contains("not JSON"));
    }
}
