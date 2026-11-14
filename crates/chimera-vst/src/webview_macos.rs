#![allow(unexpected_cfgs)]

use crate::b64::decode_bytes;
use crate::{EngineChoice, SirenParams};
use crossbeam_channel::{Receiver, Sender};
use nih_plug::prelude::{Editor, EnumParam, GuiContext, ParamSetter, ParentWindowHandle};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Protocol, Sel, BOOL, NO, YES};
use objc::{class, msg_send, sel, sel_impl, Encode, Encoding};
use serde_json::{json, Value};
use std::any::Any;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

pub const EDITOR_WIDTH: u32 = 1340;
pub const EDITOR_HEIGHT: u32 = 820;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NSPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NSSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
}

unsafe impl Encode for NSPoint {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGPoint=dd}") }
    }
}

unsafe impl Encode for NSSize {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGSize=dd}") }
    }
}

unsafe impl Encode for NSRect {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
}

unsafe extern "C" {
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

const BOOTSTRAP_JS: &str = r#"
(function() {
  if (window.__SIREN_VST__) return;
  const listeners = new Set();
  window.__SIREN_VST__ = {
    send(msg) {
      try {
        const payload = typeof msg === "string" ? msg : JSON.stringify(msg);
        window.webkit.messageHandlers.siren.postMessage(payload);
      } catch (err) {
        console.error("SIREN VST send failed:", err);
      }
    },
    onMessage(fn) {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    dispatch(msg) {
      for (const fn of listeners) {
        try {
          fn(msg);
        } catch (err) {
          console.error("SIREN VST dispatch error:", err);
        }
      }
    }
  };
})();
"#;

unsafe fn nsstring(s: &str) -> *mut Object {
    let cls = class!(NSString);
    let alloc: *mut Object = msg_send![cls, alloc];
    let init: *mut Object = msg_send![
        alloc,
        initWithBytes: s.as_ptr() as *const c_void
        length: s.len()
        encoding: 4usize // NSUTF8StringEncoding
    ];
    msg_send![init, autorelease]
}

unsafe fn nsstring_to_rust(ns: *mut Object) -> Option<String> {
    if ns.is_null() {
        return None;
    }
    let utf8: *const c_char = msg_send![ns, UTF8String];
    if utf8.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(utf8) }.to_string_lossy().into_owned())
}

struct RegisteredClasses {
    scheme_cls: &'static Class,
    bridge_cls: &'static Class,
}

fn register_objc_classes() -> &'static RegisteredClasses {
    static CLASSES: OnceLock<RegisteredClasses> = OnceLock::new();
    CLASSES.get_or_init(|| unsafe {
        let tag = register_objc_classes as *const () as usize;

        let scheme_name = format!("SirenVstSchemeHandler_{tag:x}");
        let mut scheme_decl = ClassDecl::new(&scheme_name, class!(NSObject))
            .expect("Failed to declare SirenVstSchemeHandler");
        if let Some(proto) = Protocol::get("WKURLSchemeHandler") {
            scheme_decl.add_protocol(proto);
        }
        scheme_decl.add_method(
            sel!(webView:startURLSchemeTask:),
            start_url_scheme_task as extern "C" fn(&Object, Sel, *mut Object, *mut Object),
        );
        scheme_decl.add_method(
            sel!(webView:stopURLSchemeTask:),
            stop_url_scheme_task as extern "C" fn(&Object, Sel, *mut Object, *mut Object),
        );
        let scheme_cls = scheme_decl.register();

        let bridge_name = format!("SirenVstBridgeTarget_{tag:x}");
        let mut bridge_decl = ClassDecl::new(&bridge_name, class!(NSObject))
            .expect("Failed to declare SirenVstBridgeTarget");
        if let Some(proto) = Protocol::get("WKScriptMessageHandler") {
            bridge_decl.add_protocol(proto);
        }
        bridge_decl.add_ivar::<*mut c_void>("bridge_ptr");
        bridge_decl.add_method(
            sel!(userContentController:didReceiveScriptMessage:),
            did_receive_script_message as extern "C" fn(&Object, Sel, *mut Object, *mut Object),
        );
        bridge_decl.add_method(
            sel!(onSirenTick:),
            on_siren_tick as extern "C" fn(&Object, Sel, *mut Object),
        );
        let bridge_cls = bridge_decl.register();

        RegisteredClasses {
            scheme_cls,
            bridge_cls,
        }
    })
}

extern "C" fn start_url_scheme_task(_this: &Object, _cmd: Sel, _webview: *mut Object, task: *mut Object) {
    let _ = std::panic::catch_unwind(|| unsafe {
        let pool = objc_autoreleasePoolPush();
        if !task.is_null() {
            let req: *mut Object = msg_send![task, request];
            let url: *mut Object = msg_send![req, URL];
            let path_ns: *mut Object = msg_send![url, path];
            let raw_path = nsstring_to_rust(path_ns).unwrap_or_else(|| "/index.html".to_string());

            let (status, mime, bytes): (isize, &str, &[u8]) =
                match crate::lookup_web_asset(&raw_path) {
                    Some((m, b)) => (200, m, b),
                    None => (404, "text/plain; charset=utf-8", b"Not Found"),
                };

            let keys = [
                nsstring("Content-Type"),
                nsstring("Content-Length"),
                nsstring("Access-Control-Allow-Origin"),
                nsstring("Cache-Control"),
            ];
            let len_str = bytes.len().to_string();
            let vals = [
                nsstring(mime),
                nsstring(&len_str),
                nsstring("*"),
                nsstring("no-cache"),
            ];
            let headers: *mut Object = msg_send![
                class!(NSDictionary),
                dictionaryWithObjects: vals.as_ptr()
                forKeys: keys.as_ptr()
                count: keys.len()
            ];

            let resp_alloc: *mut Object = msg_send![class!(NSHTTPURLResponse), alloc];
            let resp: *mut Object = msg_send![
                resp_alloc,
                initWithURL: url
                statusCode: status
                HTTPVersion: nsstring("HTTP/1.1")
                headerFields: headers
            ];

            let data: *mut Object = msg_send![
                class!(NSData),
                dataWithBytes: bytes.as_ptr() as *const c_void
                length: bytes.len()
            ];

            let _: () = msg_send![task, didReceiveResponse: resp];
            let _: () = msg_send![task, didReceiveData: data];
            let _: () = msg_send![task, didFinish];
            let _: () = msg_send![resp, release];
        }
        objc_autoreleasePoolPop(pool);
    });
}

extern "C" fn stop_url_scheme_task(_this: &Object, _cmd: Sel, _webview: *mut Object, _task: *mut Object) {}

extern "C" fn did_receive_script_message(
    this: &Object,
    _cmd: Sel,
    _controller: *mut Object,
    message: *mut Object,
) {
    let _ = std::panic::catch_unwind(|| unsafe {
        let pool = objc_autoreleasePoolPush();
        let ptr = *this.get_ivar::<*mut c_void>("bridge_ptr");
        if !ptr.is_null() && !message.is_null() {
            let ctx = &*(ptr as *const BridgeRuntime);
            let body: *mut Object = msg_send![message, body];
            let is_str: BOOL = msg_send![body, isKindOfClass: class!(NSString)];
            if is_str == YES {
                if let Some(json_str) = nsstring_to_rust(body) {
                    ctx.handle_js_message(&json_str);
                }
            }
        }
        objc_autoreleasePoolPop(pool);
    });
}

extern "C" fn on_siren_tick(this: &Object, _cmd: Sel, _timer: *mut Object) {
    let _ = std::panic::catch_unwind(|| unsafe {
        let pool = objc_autoreleasePoolPush();
        let ptr = *this.get_ivar::<*mut c_void>("bridge_ptr");
        if !ptr.is_null() {
            let ctx = &*(ptr as *const BridgeRuntime);
            ctx.pump_to_webview();
        }
        objc_autoreleasePoolPop(pool);
    });
}

struct BridgeRuntime {
    webview: *mut Object,
    params: Arc<SirenParams>,
    gui_context: Option<Arc<dyn GuiContext>>,
    cmd_tx: Sender<Value>,
    evt_rx: Receiver<String>,
    host_params_dirty: Arc<AtomicBool>,
    setting_from_ui: Arc<AtomicBool>,
}

impl BridgeRuntime {
    fn handle_js_message(&self, json_str: &str) {
        let Ok(val) = serde_json::from_str::<Value>(json_str) else {
            return;
        };
        let Some(ty) = val.get("type").and_then(Value::as_str) else {
            return;
        };

        match ty {
            "vstOpenUrl" => {
                if let Some(url) = val.get("url").and_then(Value::as_str) {
                    if url.starts_with("https://") || url.starts_with("http://") {
                        let _ = std::process::Command::new("open").arg(url).spawn();
                    }
                }
            }
            "vstSaveWav" => {
                if let (Some(name), Some(b64)) = (
                    val.get("name").and_then(Value::as_str),
                    val.get("b64").and_then(Value::as_str),
                ) {
                    if let Some(bytes) = decode_bytes(b64) {
                        let clean_name: String = name
                            .chars()
                            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
                            .collect();
                        let file_name = if clean_name.ends_with(".wav") {
                            clean_name
                        } else {
                            format!("{clean_name}.wav")
                        };
                        let downloads = std::env::var("HOME")
                            .map(|h| PathBuf::from(h).join("Downloads"))
                            .unwrap_or_else(|_| PathBuf::from("/tmp"));
                        let dest = downloads.join(file_name);
                        if std::fs::write(&dest, bytes).is_ok() {
                            let _ = std::process::Command::new("open")
                                .arg("-R")
                                .arg(&dest)
                                .spawn();
                        }
                    }
                }
            }
            "vstSyncHost" => {
                if let Some(state_json) = val.get("stateJson").and_then(Value::as_str) {
                    *self.params.ui_state_json.lock() = state_json.to_string();
                }
                if let Some(ctx) = &self.gui_context {
                    self.sync_params_to_host(ctx.as_ref(), &val);
                }
            }
            _ => {
                let _ = self.cmd_tx.try_send(val);
            }
        }
    }

    fn sync_params_to_host(&self, ctx: &dyn GuiContext, m: &Value) {
        self.setting_from_ui.store(true, Ordering::SeqCst);
        let setter = ParamSetter::new(ctx);

        fn set_f32(setter: &ParamSetter, param: &nih_plug::prelude::FloatParam, v: f32) {
            if (param.value() - v).abs() > 1e-4 {
                setter.begin_set_parameter(param);
                setter.set_parameter(param, v);
                setter.end_set_parameter(param);
            }
        }

        if let Some(kind_idx) = m.get("engineKind").and_then(Value::as_u64) {
            let choice = match kind_idx {
                1 => EngineChoice::Waveguide,
                2 => EngineChoice::Formant,
                3 => EngineChoice::Neural,
                _ => EngineChoice::Airway,
            };
            if self.params.engine.value() != choice {
                set_enum_param(&setter, &self.params.engine, choice);
            }
        }
        if let Some(v) = m.get("bodyFemale").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.body_female, (v as f32).clamp(0.0, 1.0));
        }
        if let Some(v) = m.get("bodyChild").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.body_child, (v as f32).clamp(0.0, 1.0));
        }
        if let Some(v) = m.get("pressure").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.pressure, (v as f32).clamp(2.0, 22.0));
        }
        if let Some(v) = m.get("breathiness").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.breathiness, (v as f32).clamp(0.0, 1.0));
        }
        if let Some(v) = m.get("tension").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.tension, (v as f32).clamp(0.0, 1.0));
        }
        if let Some(v) = m.get("brightness").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.brightness, (v as f32).clamp(0.0, 1.0));
        }
        if let Some(v) = m.get("fry").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.fry, (v as f32).clamp(0.0, 1.0));
        }
        if let Some(v) = m.get("roughness").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.roughness, (v as f32).clamp(0.0, 1.0));
        }
        if let Some(voiced) = m.get("voiced").and_then(Value::as_bool) {
            if self.params.voiced.value() != voiced {
                setter.begin_set_parameter(&self.params.voiced);
                setter.set_parameter(&self.params.voiced, voiced);
                setter.end_set_parameter(&self.params.voiced);
            }
        }
        if let Some(v) = m.get("vibratoDepth").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.vibrato_depth, (v as f32).clamp(0.0, 120.0));
        }
        if let Some(v) = m.get("vibratoRate").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.vibrato_rate, (v as f32).clamp(2.0, 10.0));
        }
        if let Some(v) = m.get("glide").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.glide, (v as f32).clamp(0.005, 0.6));
        }
        if let Some(v) = m.get("golden").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.golden_voice, (v as f32).clamp(0.0, 1.0));
        }
        if let Some(v) = m.get("velum").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.nasal, (v as f32).clamp(-0.25, 1.2));
        }
        if let Some(v) = m.get("epilarynx").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.twang, (v as f32).clamp(0.0, 1.0));
        }
        if let Some(v) = m.get("larynx").and_then(Value::as_f64) {
            set_f32(&setter, &self.params.larynx, (v as f32).clamp(-1.5, 1.5));
        }

        self.setting_from_ui.store(false, Ordering::SeqCst);
    }

    unsafe fn pump_to_webview(&self) {
        if self.webview.is_null() {
            return;
        }
        let mut batch: Vec<String> = Vec::new();
        while let Ok(msg) = self.evt_rx.try_recv() {
            batch.push(msg);
            if batch.len() >= 24 {
                break;
            }
        }

        if self.host_params_dirty.swap(false, Ordering::AcqRel) {
            let p = &self.params;
            let host_msg = json!({
                "type": "vstHostParams",
                "vowelX": p.vowel_x.value(),
                "vowelY": p.vowel_y.value(),
                "engineKind": p.engine.value().to_chimera().index(),
                "bodyFemale": p.body_female.value(),
                "bodyChild": p.body_child.value(),
                "pressure": p.pressure.value(),
                "breathiness": p.breathiness.value(),
                "tension": p.tension.value(),
                "brightness": p.brightness.value(),
                "fry": p.fry.value(),
                "roughness": p.roughness.value(),
                "voiced": p.voiced.value(),
                "vibratoDepth": p.vibrato_depth.value(),
                "vibratoRate": p.vibrato_rate.value(),
                "glide": p.glide.value(),
                "golden": p.golden_voice.value(),
                "velum": p.nasal.value(),
                "epilarynx": p.twang.value(),
                "larynx": p.larynx.value(),
            });
            if let Ok(s) = serde_json::to_string(&host_msg) {
                batch.push(s);
            }
        }

        if batch.is_empty() {
            return;
        }

        let mut js = String::with_capacity(batch.iter().map(|s| s.len() + 2).sum::<usize>() + 96);
        js.push_str("if(window.__SIREN_VST__){const _q=[");
        for (i, item) in batch.iter().enumerate() {
            if i > 0 {
                js.push(',');
            }
            js.push_str(item);
        }
        js.push_str("];for(let i=0;i<_q.length;i++)window.__SIREN_VST__.dispatch(_q[i]);}");

        let js_ns = unsafe { nsstring(&js) };
        let null_block: *mut Object = ptr::null_mut();
        let _: () = msg_send![self.webview, evaluateJavaScript: js_ns completionHandler: null_block];
    }
}

fn set_enum_param<E: nih_plug::prelude::Enum + PartialEq>(
    setter: &ParamSetter,
    param: &EnumParam<E>,
    val: E,
) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, val);
    setter.end_set_parameter(param);
}

pub struct SirenWebViewEditor {
    pub params: Arc<SirenParams>,
    pub ui_open: Arc<AtomicBool>,
    pub cmd_tx: Sender<Value>,
    pub evt_rx: Receiver<String>,
    pub host_params_dirty: Arc<AtomicBool>,
    pub setting_from_ui: Arc<AtomicBool>,
}

pub struct SpawnedWebViewHandle {
    pub webview: *mut Object,
    pub timer: *mut Object,
    pub controller: *mut Object,
    pub bridge_obj: *mut Object,
    pub scheme_obj: *mut Object,
    runtime_ptr: *mut BridgeRuntime,
    pub ui_open: Arc<AtomicBool>,
}

unsafe impl Send for SpawnedWebViewHandle {}

impl Drop for SpawnedWebViewHandle {
    fn drop(&mut self) {
        self.ui_open.store(false, Ordering::SeqCst);
        unsafe {
            let pool = objc_autoreleasePoolPush();
            if !self.timer.is_null() {
                let _: () = msg_send![self.timer, invalidate];
                let _: () = msg_send![self.timer, release];
                self.timer = ptr::null_mut();
            }
            if !self.bridge_obj.is_null() {
                (*self.bridge_obj).set_ivar::<*mut c_void>("bridge_ptr", ptr::null_mut());
            }
            if !self.controller.is_null() {
                let name = nsstring("siren");
                let _: () = msg_send![self.controller, removeScriptMessageHandlerForName: name];
                let _: () = msg_send![self.controller, removeAllUserScripts];
                let _: () = msg_send![self.controller, release];
                self.controller = ptr::null_mut();
            }
            if !self.webview.is_null() {
                let _: () = msg_send![self.webview, stopLoading];
                let _: () = msg_send![self.webview, removeFromSuperview];
                let _: () = msg_send![self.webview, release];
                self.webview = ptr::null_mut();
            }
            if !self.bridge_obj.is_null() {
                let _: () = msg_send![self.bridge_obj, release];
                self.bridge_obj = ptr::null_mut();
            }
            if !self.scheme_obj.is_null() {
                let _: () = msg_send![self.scheme_obj, release];
                self.scheme_obj = ptr::null_mut();
            }
            if !self.runtime_ptr.is_null() {
                drop(Box::from_raw(self.runtime_ptr));
                self.runtime_ptr = ptr::null_mut();
            }
            objc_autoreleasePoolPop(pool);
        }
    }
}

impl SirenWebViewEditor {
    /// Mount a native macOS `WKWebView` inside `parent_ns_view`, serving the embedded `dist/`
    /// SIREN build via `siren://localhost/index.html` and streaming 60 Hz telemetry.
    pub unsafe fn mount_into_nsview(
        &self,
        parent_ns_view: *mut Object,
        gui_context: Option<Arc<dyn GuiContext>>,
    ) -> SpawnedWebViewHandle {
        let pool = unsafe { objc_autoreleasePoolPush() };
        let classes = register_objc_classes();

        // Drain stale events from while the window was closed
        while self.evt_rx.try_recv().is_ok() {}
        self.ui_open.store(true, Ordering::SeqCst);

        let config_alloc: *mut Object = msg_send![class!(WKWebViewConfiguration), alloc];
        let config: *mut Object = msg_send![config_alloc, init];

        // Register custom `siren://` URL scheme handler backed by embedded web assets
        let scheme_alloc: *mut Object = msg_send![classes.scheme_cls, alloc];
        let scheme_obj: *mut Object = msg_send![scheme_alloc, init];
        let _: () = msg_send![config, setURLSchemeHandler: scheme_obj forURLScheme: unsafe { nsstring("siren") }];

        // UserContentController + DocumentStart bootstrap script + message handler
        let controller: *mut Object = msg_send![config, userContentController];
        let _: () = msg_send![controller, retain];

        let script_alloc: *mut Object = msg_send![class!(WKUserScript), alloc];
        let user_script: *mut Object = msg_send![
            script_alloc,
            initWithSource: unsafe { nsstring(BOOTSTRAP_JS) }
            injectionTime: 0isize // WKUserScriptInjectionTimeAtDocumentStart
            forMainFrameOnly: YES
        ];
        let _: () = msg_send![controller, addUserScript: user_script];
        let _: () = msg_send![user_script, release];

        let bounds: NSRect = if !parent_ns_view.is_null() {
            let b: NSRect = msg_send![parent_ns_view, bounds];
            if b.size.width > 10.0 && b.size.height > 10.0 {
                b
            } else {
                NSRect {
                    origin: NSPoint { x: 0.0, y: 0.0 },
                    size: NSSize {
                        width: EDITOR_WIDTH as f64,
                        height: EDITOR_HEIGHT as f64,
                    },
                }
            }
        } else {
            NSRect {
                origin: NSPoint { x: 0.0, y: 0.0 },
                size: NSSize {
                    width: EDITOR_WIDTH as f64,
                    height: EDITOR_HEIGHT as f64,
                },
            }
        };

        let wv_alloc: *mut Object = msg_send![class!(WKWebView), alloc];
        let webview: *mut Object = msg_send![wv_alloc, initWithFrame: bounds configuration: config];
        let _: () = msg_send![config, release];

        // Pure black background (zero white flash on plugin open) + autoresizing
        let no_num: *mut Object = msg_send![class!(NSNumber), numberWithBool: NO];
        let _: () = msg_send![webview, setValue: no_num forKey: unsafe { nsstring("drawsBackground") }];
        let _: () = msg_send![webview, setAllowsBackForwardNavigationGestures: NO];
        let _: () = msg_send![webview, setAllowsMagnification: NO];
        // NSViewWidthSizable (2) | NSViewHeightSizable (16) = 18
        let _: () = msg_send![webview, setAutoresizingMask: 18usize];

        if !parent_ns_view.is_null() {
            let _: () = msg_send![parent_ns_view, setWantsLayer: YES];
            let layer: *mut Object = msg_send![parent_ns_view, layer];
            if !layer.is_null() {
                let black_ns: *mut Object = msg_send![class!(NSColor), blackColor];
                let cg_black: *mut c_void = msg_send![black_ns, CGColor];
                let _: () = msg_send![layer, setBackgroundColor: cg_black];
            }
            let _: () = msg_send![parent_ns_view, addSubview: webview];
        }

        let runtime = Box::new(BridgeRuntime {
            webview,
            params: self.params.clone(),
            gui_context,
            cmd_tx: self.cmd_tx.clone(),
            evt_rx: self.evt_rx.clone(),
            host_params_dirty: self.host_params_dirty.clone(),
            setting_from_ui: self.setting_from_ui.clone(),
        });
        let runtime_ptr = Box::into_raw(runtime);

        let bridge_alloc: *mut Object = msg_send![classes.bridge_cls, alloc];
        let bridge_obj: *mut Object = msg_send![bridge_alloc, init];
        unsafe {
            (*bridge_obj).set_ivar::<*mut c_void>("bridge_ptr", runtime_ptr as *mut c_void);
        }
        let _: () = msg_send![controller, addScriptMessageHandler: bridge_obj name: unsafe { nsstring("siren") }];

        // Load `siren://localhost/index.html`
        let url: *mut Object = msg_send![
            class!(NSURL),
            URLWithString: unsafe { nsstring("siren://localhost/index.html") }
        ];
        let req: *mut Object = msg_send![class!(NSURLRequest), requestWithURL: url];
        let _: () = msg_send![webview, loadRequest: req];

        // 60 Hz timer added to NSRunLoopCommonModes so animation stays live during host mouse drags
        let timer: *mut Object = msg_send![
            class!(NSTimer),
            timerWithTimeInterval: (1.0_f64 / 60.0_f64)
            target: bridge_obj
            selector: sel!(onSirenTick:)
            userInfo: ptr::null_mut::<Object>()
            repeats: YES
        ];
        let _: () = msg_send![timer, retain];
        let run_loop: *mut Object = msg_send![class!(NSRunLoop), mainRunLoop];
        let common_modes = CString::new("kCFRunLoopCommonModes").unwrap();
        let mode_ns: *mut Object = msg_send![class!(NSString), stringWithUTF8String: common_modes.as_ptr()];
        let _: () = msg_send![run_loop, addTimer: timer forMode: mode_ns];

        unsafe { objc_autoreleasePoolPop(pool) };

        SpawnedWebViewHandle {
            webview,
            timer,
            controller,
            bridge_obj,
            scheme_obj,
            runtime_ptr,
            ui_open: self.ui_open.clone(),
        }
    }
}

impl Editor for SirenWebViewEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn Any + Send> {
        let ns_view = match parent {
            ParentWindowHandle::AppKitNsView(ptr) => ptr as *mut Object,
            _ => ptr::null_mut(),
        };
        let handle = unsafe { self.mount_into_nsview(ns_view, Some(context)) };
        Box::new(handle)
    }

    fn size(&self) -> (u32, u32) {
        (EDITOR_WIDTH, EDITOR_HEIGHT)
    }

    fn set_scale_factor(&self, _factor: f32) -> bool {
        true
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        if !self.setting_from_ui.load(Ordering::SeqCst) {
            self.host_params_dirty.store(true, Ordering::Release);
        }
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}

    fn param_values_changed(&self) {
        if !self.setting_from_ui.load(Ordering::SeqCst) {
            self.host_params_dirty.store(true, Ordering::Release);
        }
    }
}
