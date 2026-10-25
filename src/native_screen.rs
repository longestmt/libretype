// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Single-window screenshots and local OCR. No image or recognized text is persisted.
use super::*;
use crate::{
    background::screen_allowed,
    screen_context::{self, Line, Window},
};
use crate::native_blocks::{PairBlock, Signature};

#[link(name = "ScreenCaptureKit", kind = "framework")]
extern "C" {}
#[link(name = "Vision", kind = "framework")]
extern "C" {}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGWindowListCopyWindowInfo(options: u32, relative: u32) -> CF;
    fn CGRectMakeWithDictionaryRepresentation(dict: CF, rect: *mut NSRect) -> bool;
    static kCGWindowNumber: CF;
    static kCGWindowOwnerPID: CF;
    static kCGWindowLayer: CF;
    static kCGWindowBounds: CF;
    static kCGWindowIsOnscreen: CF;
    fn CGImageGetWidth(image: CF) -> usize;
    fn CGImageGetHeight(image: CF) -> usize;
    fn CGColorSpaceCreateDeviceRGB() -> CF;
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits: usize,
        row: usize,
        space: CF,
        info: u32,
    ) -> CF;
    fn CGBitmapContextCreateImage(context: CF) -> CF;
    fn CGContextSetRGBFillColor(context: CF, r: f64, g: f64, b: f64, a: f64);
    fn CGContextFillRect(context: CF, rect: NSRect);
    fn CGPathCreateWithRect(rect: NSRect, transform: CF) -> CF;
}
#[link(name = "CoreText", kind = "framework")]
extern "C" {
    fn CTFontCreateWithName(name: CF, size: f64, matrix: CF) -> CF;
    static kCTFontAttributeName: CF;
    fn CTFramesetterCreateWithAttributedString(text: CF) -> CF;
    fn CTFramesetterCreateFrame(setter: CF, range: Range, path: CF, attributes: CF) -> CF;
    fn CTFrameDraw(frame: CF, context: CF);
}

pub(super) fn supported() -> bool {
    objc::runtime::Class::get("SCScreenshotManager").is_some()
}
pub(super) unsafe fn permitted() -> bool {
    CGPreflightScreenCaptureAccess()
}
pub(super) unsafe fn request_permission() {
    CGRequestScreenCaptureAccess();
}

pub(super) struct Image(CF);
// CGImage is immutable and retained explicitly. No mutable Cocoa UI object crosses threads.
unsafe impl Send for Image {}
impl Drop for Image {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}

unsafe fn active_window(ctx: &Context) -> Option<Window> {
    let list = OwnedCf(CGWindowListCopyWindowInfo(1 | 16, 0)); // on-screen, excluding desktop
    if list.0.is_null() {
        return None;
    }
    let array = list.0 as id;
    let count: usize = msg_send![array, count];
    let mut windows = Vec::new();
    for index in 0..count.min(1024) {
        let dict: id = msg_send![array,objectAtIndex:index];
        let number = |key: CF| -> i32 {
            let value = CFDictionaryGetValue(dict as CF, key) as id;
            msg_send![value, intValue]
        };
        let bounds = CFDictionaryGetValue(dict as CF, kCGWindowBounds);
        let mut rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
        if bounds.is_null() || !CGRectMakeWithDictionaryRepresentation(bounds, &mut rect) {
            continue;
        }
        windows.push(Window {
            id: number(kCGWindowNumber) as u32,
            pid: number(kCGWindowOwnerPID),
            layer: number(kCGWindowLayer),
            visible: number(kCGWindowIsOnscreen) != 0,
            bounds: [
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
            ],
        });
    }
    screen_context::choose(&windows, ctx.pid, [ctx.caret.origin.x, ctx.caret.origin.y])
}

struct ImageWork {
    id: u64,
    image: Image,
    caret: [f64; 2],
}
struct Reply {
    id: u64,
    result: Result<String, &'static str>,
}
pub(super) enum Work {
    Completion(CompletionRequest),
    Choices(crate::alternatives::Request),
    Preview(CompletionRequest),
}
impl Work {
    fn id(&self) -> u64 {
        match self {
            Self::Completion(r) | Self::Preview(r) => r.id,
            Self::Choices(r) => r.id,
        }
    }
    fn set_text(&mut self, text: &str) {
        match self {
            Self::Completion(r) | Self::Preview(r) => r.background.set_screen(text),
            Self::Choices(r) => r.background.set_screen(text),
        }
    }
}
struct Pending {
    work: Work,
    config: Config,
    window: Option<Window>,
    started: Instant,
}
pub(super) struct Engine {
    images: mpsc::SyncSender<ImageWork>,
    result_sender: mpsc::Sender<Reply>,
    replies: mpsc::Receiver<Reply>,
    pending: Option<Pending>,
}
impl Engine {
    pub(super) fn new(epoch: Arc<AtomicU64>) -> Self {
        let (images, rx) = mpsc::sync_channel::<ImageWork>(1);
        let (out, replies) = mpsc::channel();
        let result_sender = out.clone();
        std::thread::spawn(move || {
            while let Ok(mut work) = rx.recv() {
                while let Ok(new) = rx.try_recv() {
                    work = new;
                }
                if epoch.load(Ordering::SeqCst) != work.id {
                    continue;
                }
                let result = unsafe { recognize(&work.image, work.caret) };
                if epoch.load(Ordering::SeqCst) == work.id {
                    let _ = out.send(Reply {
                        id: work.id,
                        result,
                    });
                }
            }
        });
        Self {
            images,
            result_sender,
            replies,
            pending: None,
        }
    }
    pub(super) fn clear(&mut self) {
        self.pending = None;
    }
    pub(super) fn waiting(&self) -> bool {
        self.pending.is_some()
    }
}

/// Called only after explicit opt-in, permission preflight and full focused-field capture.
unsafe fn start_capture(
    window: Window,
    caret: [f64; 2],
    id: u64,
    epoch: Arc<AtomicU64>,
    images: mpsc::SyncSender<ImageWork>,
    out: mpsc::Sender<Reply>,
) {
    let completion=PairBlock::new(Signature::Objects, move |content,error| {
        let content=content.cast::<Object>(); let error=error.cast::<Object>();
        if epoch.load(Ordering::SeqCst)!=id { return; }
        let pool:id=msg_send![class!(NSAutoreleasePool),new];
        let capture=|| {
            if content==nil || error!=nil || !CGPreflightScreenCaptureAccess() || IsSecureEventInputEnabled() || front_identity().0!=window.pid { return false; }
            let windows:id=msg_send![content,windows]; let count:usize=msg_send![windows,count];
            let mut selected=nil;
            for index in 0..count.min(1024) {
                let candidate:id=msg_send![windows,objectAtIndex:index];
                let number:u32=msg_send![candidate,windowID];
                if number!=window.id { continue; }
                let owner:id=msg_send![candidate,owningApplication]; let pid:i32=msg_send![owner,processID];
                let visible:BOOL=msg_send![candidate,isOnScreen]; let frame:NSRect=msg_send![candidate,frame];
                if pid!=window.pid || visible!=YES || [frame.origin.x,frame.origin.y,frame.size.width,frame.size.height].iter().zip(window.bounds).any(|(a,b)|(a-b).abs()>2.) { return false; }
                selected=candidate; break;
            }
            if selected==nil || epoch.load(Ordering::SeqCst)!=id { return false; }
            let filter:id=msg_send![class!(SCContentFilter),alloc];
            let filter:id=msg_send![filter,initWithDesktopIndependentWindow:selected];
            let config:id=msg_send![class!(SCStreamConfiguration),new];
            let (width,height)=window.pixels();
            let _:()=msg_send![config,setWidth:width]; let _:()=msg_send![config,setHeight:height];
            let _:()=msg_send![config,setShowsCursor:NO]; let _:()=msg_send![config,setScalesToFit:YES];
            let _:()=msg_send![config,setIgnoreShadowsSingleWindow:YES];
            let children:BOOL=msg_send![config,respondsToSelector:sel!(setIncludeChildWindows:)];
            if children==YES { let _:()=msg_send![config,setIncludeChildWindows:NO]; }
            let epoch2=epoch.clone(); let images2=images.clone(); let out2=out.clone();
            let done=PairBlock::new(Signature::ImageAndError, move |image,error| {
                let image=image as CF; let error=error.cast::<Object>();
                if epoch2.load(Ordering::SeqCst)!=id { return; }
                if image.is_null() || error!=nil {
                    let _=out2.send(Reply{id,result:Err("Window capture unavailable")}); return;
                }
                let retained=Image(CFRetain(image));
                if images2.try_send(ImageWork{id,image:retained,caret}).is_err() {
                    let _=out2.send(Reply{id,result:Err("OCR worker busy")});
                }
            });
            let _:()=msg_send![class!(SCScreenshotManager),captureImageWithFilter:filter configuration:config completionHandler:done.as_ptr()];
            let _:()=msg_send![config,release]; let _:()=msg_send![filter,release];
            true
        };
        if !capture() { let _=out.send(Reply{id,result:Err("Window changed or capture unavailable")}); }
        let _:()=msg_send![pool,drain];
    });
    let _: () = msg_send![class!(SCShareableContent),getShareableContentExcludingDesktopWindows:YES onScreenWindowsOnly:YES completionHandler:completion.as_ptr()];
}

unsafe fn recognize(image: &Image, caret: [f64; 2]) -> Result<String, &'static str> {
    let (w, h) = (CGImageGetWidth(image.0), CGImageGetHeight(image.0));
    if w == 0 || h == 0 || w.saturating_mul(h) > 4096 * 3072 {
        return Err("Image exceeds OCR limits");
    }
    let pool: id = msg_send![class!(NSAutoreleasePool), new];
    let request: id = msg_send![class!(VNRecognizeTextRequest), new];
    let _: () = msg_send![request,setRecognitionLevel:0isize];
    let _: () = msg_send![request,setUsesLanguageCorrection:YES];
    let _: () = msg_send![request,setAutomaticallyDetectsLanguage:YES];
    let options: id = msg_send![class!(NSDictionary), dictionary];
    let handler: id = msg_send![class!(VNImageRequestHandler), alloc];
    let handler: id = msg_send![handler,initWithCGImage:image.0 options:options];
    let requests: id = msg_send![class!(NSArray),arrayWithObject:request];
    let mut error: id = nil;
    let ok: BOOL = msg_send![handler,performRequests:requests error:&mut error];
    let mut lines = Vec::new();
    if ok == YES && error == nil {
        let results: id = msg_send![request, results];
        let count: usize = msg_send![results, count];
        for index in 0..count.min(256) {
            let observation: id = msg_send![results,objectAtIndex:index];
            let candidates: id = msg_send![observation,topCandidates:1usize];
            let candidate: id = msg_send![candidates, firstObject];
            if candidate == nil {
                continue;
            }
            let text = ns_text(msg_send![candidate, string]);
            let rect: NSRect = msg_send![observation, boundingBox];
            lines.push(Line {
                text: text.chars().take(256).collect(),
                bounds: [
                    rect.origin.x,
                    rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                ],
            });
        }
    }
    let _: () = msg_send![handler, release];
    let _: () = msg_send![request, release];
    let _: () = msg_send![pool, drain];
    if ok != YES || error != nil {
        Err("Local text recognition failed")
    } else {
        Ok(screen_context::nearby_text(lines, caret))
    }
}

/// Render only the caller-supplied sample with CoreText. Never reads a display.
pub(super) unsafe fn sample_image(text: &str) -> Option<Image> {
    let space = OwnedCf(CGColorSpaceCreateDeviceRGB());
    let context = OwnedCf(CGBitmapContextCreate(
        ptr::null_mut(),
        1200,
        300,
        8,
        1200 * 4,
        space.0,
        1,
    ));
    if context.0.is_null() {
        return None;
    }
    CGContextSetRGBFillColor(context.0, 1., 1., 1., 1.);
    CGContextFillRect(
        context.0,
        NSRect::new(NSPoint::new(0., 0.), NSSize::new(1200., 300.)),
    );
    CGContextSetRGBFillColor(context.0, 0., 0., 0., 1.);
    let font_name = cfstr("Helvetica");
    let font = OwnedCf(CTFontCreateWithName(font_name.0, 32., ptr::null()));
    let attributes: id = msg_send![class!(NSDictionary),dictionaryWithObject:font.0 as id forKey:kCTFontAttributeName as id];
    let sample: String = text.chars().take(300).collect();
    let sample = ns(&sample);
    let attributed: id = msg_send![class!(NSAttributedString), alloc];
    let attributed: id = msg_send![attributed,initWithString:sample attributes:attributes];
    let setter = OwnedCf(CTFramesetterCreateWithAttributedString(attributed as CF));
    let path = OwnedCf(CGPathCreateWithRect(
        NSRect::new(NSPoint::new(30., 20.), NSSize::new(1140., 260.)),
        ptr::null(),
    ));
    let frame = OwnedCf(CTFramesetterCreateFrame(
        setter.0,
        Range {
            location: 0,
            length: 0,
        },
        path.0,
        ptr::null(),
    ));
    CTFrameDraw(frame.0, context.0);
    let _: () = msg_send![attributed, release];
    let _: () = msg_send![sample, release];
    let image = CGBitmapContextCreateImage(context.0);
    (!image.is_null()).then(|| Image(image))
}

impl State {
    unsafe fn dispatch_work(&mut self, config: Config, work: Work) -> bool {
        // Only describe actual editor jobs accepted by the inference queue.
        // Never retain profile text, vocabulary or source identity in diagnostics.
        let attached = match &work {
            Work::Completion(request) => Some(request.profile.is_some()),
            Work::Choices(request) => Some(request.profile.is_some()),
            Work::Preview(_) => None,
        };
        let sent = match work {
            Work::Completion(request) | Work::Preview(request) => {
                self.requests.try_send((config, request)).is_ok()
            }
            Work::Choices(request) => self.choice_requests.try_send((config, request)).is_ok(),
        };
        if sent {
            self.request_profile_attached = attached;
            if attached == Some(true) {
                self.personalized_requests = self.personalized_requests.saturating_add(1);
            }
        }
        sent
    }
    pub(super) unsafe fn submit_with_screen(&mut self, work: Work) -> bool {
        if let Work::Choices(request) = &work {
            if !request.valid()
                || (!request.is_synonym()
                    && !request.started_word().is_empty()
                    && request.candidates.is_empty())
            {
                return self.dispatch_work(self.config.clone(), work);
            }
        }
        if let Some(ctx) = &self.context {
            if screen_allowed(&self.config, &ctx.app, ctx.website.as_deref())
                && supported()
                && permitted()
                && capture_selected(&self.config, 0.08, true).is_some_and(|now| ctx.same(&now))
            {
                if let Some(window) = active_window(ctx) {
                    let caret = window
                        .normalized_caret([ctx.caret.origin.x, ctx.caret.origin.y])
                        .unwrap();
                    let id = work.id();
                    self.screen.pending = Some(Pending {
                        work,
                        config: self.config.clone(),
                        window: Some(window),
                        started: Instant::now(),
                    });
                    start_capture(
                        window,
                        caret,
                        id,
                        self.epoch.clone(),
                        self.screen.images.clone(),
                        self.screen.result_sender.clone(),
                    );
                    return true;
                }
            }
        }
        self.dispatch_work(self.config.clone(), work)
    }
    pub(super) unsafe fn preview_screen(
        &mut self,
        config: Config,
        request: CompletionRequest,
        sample: &str,
    ) -> bool {
        let Some(image) = sample_image(sample) else {
            return false;
        };
        let id = request.id;
        if self
            .screen
            .images
            .try_send(ImageWork {
                id,
                image,
                caret: [0.5, 0.5],
            })
            .is_err()
        {
            return false;
        }
        self.screen.pending = Some(Pending {
            work: Work::Preview(request),
            config,
            window: None,
            started: Instant::now(),
        });
        true
    }
    pub(super) unsafe fn process_screen_replies(&mut self) {
        while let Ok(reply) = self.screen.replies.try_recv() {
            if reply.id != self.epoch.load(Ordering::SeqCst)
                || self
                    .screen
                    .pending
                    .as_ref()
                    .is_none_or(|p| p.work.id() != reply.id)
            {
                continue;
            }
            self.finish_screen(reply.result);
        }
        if self
            .screen
            .pending
            .as_ref()
            .is_some_and(|p| p.started.elapsed() > Duration::from_secs(4))
        {
            self.finish_screen(Err("Screen context timed out"));
        }
    }
    unsafe fn finish_screen(&mut self, result: Result<String, &'static str>) {
        let Some(mut pending) = self.screen.pending.take() else {
            return;
        };
        if pending.work.id() != self.epoch.load(Ordering::SeqCst) {
            return;
        }
        let preview = matches!(pending.work, Work::Preview(_));
        if !preview {
            let valid = match (&self.context, capture_selected(&self.config, 0.08, true)) {
                (Some(old), Some(now)) => {
                    permitted() && old.same(&now) && active_window(&now) == pending.window
                }
                _ => false,
            };
            if !valid {
                self.clear();
                return;
            }
        }
        match result {
            Ok(text) => {
                if preview && text.trim().is_empty() {
                    self.preview_epoch = None;
                    self.background_ui
                        .ocr_error("No readable text in the sample image.");
                    return;
                }
                if preview {
                    self.background_ui.ocr_message(&text);
                }
                pending.work.set_text(&text);
            }
            Err(error) if preview => {
                self.preview_epoch = None;
                self.background_ui.ocr_error(error);
                return;
            }
            Err(_) => {} // Ordinary completions still work if screen context is unavailable.
        }
        if !self.dispatch_work(pending.config, pending.work) {
            if preview {
                self.preview_epoch = None;
                self.background_ui
                    .ocr_error("Model worker busy. Try preview again.");
            }
            self.requested = false;
            self.choice_progress = choices::Progress::Untried;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "synthetic image, OCR worker cancellation, and downloaded local model; no screen capture"]
    fn synthetic_ocr_and_model_live() {
        let epoch = Arc::new(AtomicU64::new(2));
        let engine = Engine::new(epoch.clone());
        unsafe {
            let pool: id = msg_send![class!(NSAutoreleasePool), new];
            let stale = sample_image("Discard this stale sample.").unwrap();
            let current = sample_image("The meeting is on Friday at Blue Heron Library.").unwrap();
            engine
                .images
                .send(ImageWork {
                    id: 1,
                    image: stale,
                    caret: [0.5, 0.5],
                })
                .unwrap();
            engine
                .images
                .send(ImageWork {
                    id: 2,
                    image: current,
                    caret: [0.5, 0.5],
                })
                .unwrap();
            let _: () = msg_send![pool, drain];
        }
        let reply = engine
            .replies
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(reply.id, 2);
        let text = reply.result.unwrap();
        assert!(text.contains("Blue Heron Library"));
        assert!(engine.replies.try_recv().is_err());
        let mut config = Config::default();
        config.background.screen = true;
        let mut reference = crate::background::Background::default();
        reference.set_screen(&text);
        let request = CompletionRequest {
            id: 2,
            prefix: "The meeting is at ".into(),
            suffix: String::new(),
            app_id: "org.libretypist.test".into(),
            website: None,
            profile: None,
            background: reference,
        };
        let output = model::generate(&config, &request, &epoch).unwrap();
        println!("Synthetic OCR completion: {output}");
        assert!(output.contains("Blue Heron Library"));
    }

    #[test]
    #[ignore = "runs macOS Vision on an in-memory synthetic image; never captures the screen"]
    fn synthetic_ocr_live() {
        unsafe {
            let pool: id = msg_send![class!(NSAutoreleasePool), new];
            let image = sample_image("The meeting is on Friday at Blue Heron Library.").unwrap();
            let text = recognize(&image, [0.5, 0.5]).unwrap();
            println!("Synthetic OCR: {text}");
            assert!(text.contains("Blue Heron Library"));
            assert!(text.contains("Friday"));
            let _: () = msg_send![pool, drain];
        }
    }
}
