//! Native macOS WKWebView verification harness for `SirenWebViewEditor`.

#![allow(unexpected_cfgs)]

#[cfg(target_os = "macos")]
fn main() {
    use chimera_vst::dsp::DspCore;
    use chimera_vst::webview_macos::{
        NSPoint, NSRect, NSSize, SirenWebViewEditor, EDITOR_HEIGHT, EDITOR_WIDTH,
    };
    use chimera_vst::SirenParams;
    use crossbeam_channel::bounded;
    use objc::runtime::{Object, BOOL, NO, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::path::PathBuf;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};
    use std::time::{Duration, Instant};

    unsafe extern "C" {
        static _NSConcreteGlobalBlock: *const c_void;
    }

    #[repr(C)]
    struct BlockDescriptor {
        reserved: usize,
        size: usize,
    }

    #[repr(C)]
    struct GlobalBlock2 {
        isa: *const c_void,
        flags: i32,
        reserved: i32,
        invoke: unsafe extern "C" fn(*mut GlobalBlock2, *mut Object, *mut Object),
        descriptor: *const BlockDescriptor,
    }

    unsafe impl Sync for GlobalBlock2 {}

    static DESCRIPTOR: BlockDescriptor = BlockDescriptor {
        reserved: 0,
        size: std::mem::size_of::<GlobalBlock2>(),
    };

    static SNAPSHOT_DONE: AtomicBool = AtomicBool::new(false);
    static JS_RESULT: OnceLock<String> = OnceLock::new();
    static JS_DONE: AtomicBool = AtomicBool::new(false);

    unsafe fn nsstring(s: &str) -> *mut Object {
        let alloc: *mut Object = msg_send![class!(NSString), alloc];
        let init: *mut Object = msg_send![
            alloc,
            initWithBytes: s.as_ptr() as *const c_void
            length: s.len()
            encoding: 4usize
        ];
        msg_send![init, autorelease]
    }

    unsafe extern "C" fn on_js_eval(_block: *mut GlobalBlock2, result: *mut Object, err: *mut Object) {
        if !err.is_null() {
            let desc: *mut Object = msg_send![err, localizedDescription];
            let utf8: *const c_char = msg_send![desc, UTF8String];
            if !utf8.is_null() {
                let s = unsafe { CStr::from_ptr(utf8) }.to_string_lossy().into_owned();
                let _ = JS_RESULT.set(format!("ERR:{s}"));
            }
        } else if !result.is_null() {
            let desc: *mut Object = msg_send![result, description];
            let utf8: *const c_char = msg_send![desc, UTF8String];
            if !utf8.is_null() {
                let s = unsafe { CStr::from_ptr(utf8) }.to_string_lossy().into_owned();
                let _ = JS_RESULT.set(s);
            }
        }
        JS_DONE.store(true, Ordering::SeqCst);
    }

    unsafe extern "C" fn on_snapshot(_block: *mut GlobalBlock2, image: *mut Object, err: *mut Object) {
        if !err.is_null() {
            let desc: *mut Object = msg_send![err, localizedDescription];
            let utf8: *const c_char = msg_send![desc, UTF8String];
            if !utf8.is_null() {
                eprintln!(
                    "takeSnapshot error: {}",
                    unsafe { CStr::from_ptr(utf8) }.to_string_lossy()
                );
            }
            SNAPSHOT_DONE.store(true, Ordering::SeqCst);
            return;
        }
        if !image.is_null() {
            let tiff: *mut Object = msg_send![image, TIFFRepresentation];
            if !tiff.is_null() {
                let rep: *mut Object = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
                if !rep.is_null() {
                    let empty_props: *mut Object = msg_send![class!(NSDictionary), dictionary];
                    let png_data: *mut Object = msg_send![
                        rep,
                        representationUsingType: 4usize
                        properties: empty_props
                    ];
                    if !png_data.is_null() {
                        let len: usize = msg_send![png_data, length];
                        let bytes: *const u8 = msg_send![png_data, bytes];
                        let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
                        let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out");
                        let _ = std::fs::create_dir_all(&out_dir);
                        let out_path = out_dir.join("siren-vst-3d.png");
                        std::fs::write(&out_path, slice).expect("failed to write siren-vst-3d.png");
                        println!("Saved snapshot: {} ({} bytes)", out_path.display(), len);
                    }
                }
            }
        }
        SNAPSHOT_DONE.store(true, Ordering::SeqCst);
    }

    unsafe {
        chimera::init();
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let _: BOOL = msg_send![app, setActivationPolicy: 1isize];
        let _: () = msg_send![app, finishLaunching];

        let rect = NSRect {
            origin: NSPoint { x: 80.0, y: 80.0 },
            size: NSSize {
                width: EDITOR_WIDTH as f64,
                height: EDITOR_HEIGHT as f64,
            },
        };
        let win_alloc: *mut Object = msg_send![class!(NSWindow), alloc];
        let window: *mut Object = msg_send![
            win_alloc,
            initWithContentRect: rect
            styleMask: 1usize
            backing: 2usize
            defer: NO
        ];
        let _: () = msg_send![window, setIgnoresMouseEvents: YES];
        let _: () = msg_send![window, makeKeyAndOrderFront: ptr::null_mut::<Object>()];

        let content_view: *mut Object = msg_send![window, contentView];

        let params = Arc::new(SirenParams::default());
        let ui_open = Arc::new(AtomicBool::new(false));
        let host_params_dirty = Arc::new(AtomicBool::new(false));
        let setting_from_ui = Arc::new(AtomicBool::new(false));
        let (cmd_tx, cmd_rx) = bounded(512);
        let (evt_tx, evt_rx) = bounded(256);

        let editor = SirenWebViewEditor {
            params: params.clone(),
            ui_open: ui_open.clone(),
            cmd_tx: cmd_tx.clone(),
            evt_rx,
            host_params_dirty,
            setting_from_ui,
        };

        let handle = editor.mount_into_nsview(content_view, None);

        // Inject error + rAF counter hook right away
        let diag_js = r#"
            window.__DIAG__ = { errs: [], rafs: 0 };
            window.addEventListener('error', e => window.__DIAG__.errs.push('ERR:' + e.message));
            window.addEventListener('unhandledrejection', e => window.__DIAG__.errs.push('REJ:' + String(e.reason)));
            const _raf = () => { window.__DIAG__.rafs++; requestAnimationFrame(_raf); };
            requestAnimationFrame(_raf);
        "#;
        let _: () = msg_send![
            handle.webview,
            evaluateJavaScript: nsstring(diag_js)
            completionHandler: ptr::null_mut::<c_void>()
        ];

        let stop_audio = Arc::new(AtomicBool::new(false));
        let stop_clone = stop_audio.clone();
        let params_audio = params.clone();
        let ui_open_audio = ui_open.clone();
        let audio_thread = std::thread::spawn(move || {
            let mut core = DspCore::new(48_000.0, 512, ui_open_audio, cmd_rx, evt_tx);
            let mut left = vec![0.0f32; 256];
            let mut right = vec![0.0f32; 256];
            let start = Instant::now();
            let mut note_triggered = false;
            while !stop_clone.load(Ordering::Relaxed) {
                core.drain_ui_commands("");
                if !note_triggered && start.elapsed() > Duration::from_millis(650) {
                    note_triggered = true;
                    core.note_on(52, 0.92, &params_audio);
                }
                core.render_slice(&mut left, &mut right, 1.0);
                std::thread::sleep(Duration::from_micros(5_333));
            }
        });

        let run_loop: *mut Object = msg_send![class!(NSRunLoop), mainRunLoop];
        let mode_c = CString::new("kCFRunLoopDefaultMode").unwrap();
        let mode_ns: *mut Object = msg_send![class!(NSString), stringWithUTF8String: mode_c.as_ptr()];

        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
        let pump_until = Instant::now() + Duration::from_millis(2800);
        while Instant::now() < pump_until {
            let step_date: *mut Object =
                msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.016f64];
            loop {
                let ev: *mut Object = msg_send![
                    app,
                    nextEventMatchingMask: usize::MAX
                    untilDate: ptr::null_mut::<Object>()
                    inMode: mode_ns
                    dequeue: YES
                ];
                if ev.is_null() {
                    break;
                }
                let _: () = msg_send![app, sendEvent: ev];
            }
            let _: () = msg_send![app, updateWindows];
            let _: () = msg_send![run_loop, runMode: mode_ns beforeDate: step_date];
        }

        let js_block = GlobalBlock2 {
            isa: _NSConcreteGlobalBlock,
            flags: 1 << 28,
            reserved: 0,
            invoke: on_js_eval,
            descriptor: &DESCRIPTOR,
        };
        let probe_js = r#"
            (() => {
                const c3 = document.querySelector('.anatomy.three canvas');
                let nonBlack3d = 0;
                if (c3 && c3.width > 0 && c3.height > 0) {
                    const ctx = c3.getContext('2d');
                    const img = ctx.getImageData(Math.floor(c3.width * 0.25), Math.floor(c3.height * 0.2), Math.floor(c3.width * 0.5), Math.floor(c3.height * 0.6)).data;
                    for (let i = 0; i < img.length; i += 16) {
                        if (img[i] + img[i+1] + img[i+2] > 25) nonBlack3d++;
                    }
                }
                return JSON.stringify({
                    vstRoot: document.documentElement.classList.contains('siren-vst'),
                    hasStartPrompt: !!document.querySelector('.start-prompt'),
                    noWebgl: !!document.querySelector('.no-webgl'),
                    threeWrapRect: document.querySelector('.anatomy.three')?.getBoundingClientRect() ?? null,
                    c3Width: c3?.width ?? 0,
                    c3Height: c3?.height ?? 0,
                    nonBlack3d,
                    allCanvases: Array.from(document.querySelectorAll('canvas')).map(c => [c.parentElement?.className, c.width, c.height]),
                    hidden: document.hidden, vis: document.visibilityState,
                });
            })()
        "#;
        let _: () = msg_send![
            handle.webview,
            evaluateJavaScript: nsstring(probe_js)
            completionHandler: &js_block as *const GlobalBlock2 as *const c_void
        ];

        let js_deadline = Instant::now() + Duration::from_secs(2);
        while !JS_DONE.load(Ordering::SeqCst) && Instant::now() < js_deadline {
            let step_date: *mut Object =
                msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.01f64];
            let _: () = msg_send![run_loop, runMode: mode_ns beforeDate: step_date];
        }
        let dom_report = JS_RESULT.get().cloned().unwrap_or_default();
        println!("DOM Probe: {dom_report}");

        let snap_block = GlobalBlock2 {
            isa: _NSConcreteGlobalBlock,
            flags: 1 << 28,
            reserved: 0,
            invoke: on_snapshot,
            descriptor: &DESCRIPTOR,
        };
        let _: () = msg_send![
            handle.webview,
            takeSnapshotWithConfiguration: ptr::null_mut::<Object>()
            completionHandler: &snap_block as *const GlobalBlock2 as *const c_void
        ];

        let snap_deadline = Instant::now() + Duration::from_secs(3);
        while !SNAPSHOT_DONE.load(Ordering::SeqCst) && Instant::now() < snap_deadline {
            let step_date: *mut Object =
                msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.016f64];
            let _: () = msg_send![run_loop, runMode: mode_ns beforeDate: step_date];
        }

        stop_audio.store(true, Ordering::Relaxed);
        let _ = audio_thread.join();
        drop(handle);
        let _: () = msg_send![window, close];

        assert!(
            dom_report.contains("\"noWebgl\":false") && !dom_report.contains("\"nonBlack3d\":0"),
            "Expected lit 3D head pixels in .anatomy.three canvas! Got: {dom_report}"
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("snapshot_vst_ui is macOS-only");
}
