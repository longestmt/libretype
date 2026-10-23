// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Main-app login registration. macOS is the source of truth; no mirrored flag.
use super::*;
use objc::runtime::Class;

#[link(name = "ServiceManagement", kind = "framework")]
extern "C" {
    #[link_name = "OBJC_CLASS_$_SMAppService"]
    static SERVICE_CLASS: Class;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Off,
    Enabled,
    Approval,
    NotFound,
    Unknown,
}
impl Status {
    fn from_raw(value: isize) -> Self {
        match value {
            0 => Self::Off,
            1 => Self::Enabled,
            2 => Self::Approval,
            3 => Self::NotFound,
            _ => Self::Unknown,
        }
    }
    fn checkbox(self) -> isize {
        match self {
            Self::Enabled => 1,
            Self::Approval => -1,
            _ => 0,
        }
    }
    fn message(self) -> &'static str {
        match self{
        Self::Off=>"Off. LibreType is not registered to open at login.",
        Self::Enabled=>"Enabled in macOS. LibreType is registered to open at login.",
        Self::Approval=>"Registered, but macOS approval is required. Open Login Items settings to allow LibreType.",
            Self::NotFound=>"No login registration was found for this app. Enable the option to ask macOS to register it.",
        Self::Unknown=>"Login status is unavailable. Check macOS Login Items settings.",
    }
    }
}
unsafe fn service() -> id {
    msg_send![&SERVICE_CLASS, mainAppService]
}
unsafe fn status() -> Status {
    let item = service();
    if item == nil {
        return Status::Unknown;
    }
    let code: isize = msg_send![item, status];
    Status::from_raw(code)
}

pub(super) struct StartupUi {
    window: id,
    checkbox: id,
    status: id,
    result: id,
    last_poll: Instant,
    message: String,
}
impl StartupUi {
    pub(super) unsafe fn new(target: id) -> Self {
        let window: id = msg_send![class!(NSWindow), alloc];
        let window: id = msg_send![window,initWithContentRect:rect(0.,0.,550.,340.) styleMask:3usize backing:2usize defer:NO];
        set_title(window, "LibreType — Startup");
        let _: () = msg_send![window,setReleasedWhenClosed:NO];
        let _: () = msg_send![window, center];
        let view: id = msg_send![window, contentView];
        label(view, "Startup", rect(24., 282., 500., 38.), 26.);
        let checkbox: id = msg_send![class!(NSButton), alloc];
        let checkbox: id = msg_send![checkbox,initWithFrame:rect(24.,230.,500.,30.)];
        let _: () = msg_send![checkbox,setButtonType:3usize];
        let _: () = msg_send![checkbox,setAllowsMixedState:YES];
        set_title(checkbox, "Open LibreType at login");
        let _: () = msg_send![checkbox,setTarget:target];
        let _: () = msg_send![checkbox,setAction:sel!(toggleLogin:)];
        let _: () = msg_send![view,addSubview:checkbox];
        let _: () = msg_send![checkbox, release];
        let status = label(view, "", rect(24., 168., 500., 54.), 13.);
        label(view,"Changes apply immediately. Login launches stay in the menu bar.\nOllama must also be running for writing suggestions.\nmacOS can require approval or disable a login item in System Settings.",rect(24.,93.,500.,66.),12.);
        button(
            view,
            "Open Login Items settings…",
            rect(20., 50., 300., 32.),
            target,
            sel!(loginSettings:),
        );
        let result = label(view, "", rect(24., 8., 500., 37.), 12.);
        Self {
            window,
            checkbox,
            status,
            result,
            last_poll: Instant::now() - Duration::from_secs(2),
            message: String::new(),
        }
    }
    pub(super) unsafe fn poll(&mut self) {
        let visible: BOOL = msg_send![self.window, isVisible];
        if visible != YES || self.last_poll.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.refresh();
    }
    unsafe fn refresh(&mut self) {
        self.last_poll = Instant::now();
        let state = status();
        let _: () = msg_send![self.checkbox,setState:state.checkbox()];
        set_text(self.status, state.message());
        set_text(self.result, &self.message);
    }
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
unsafe fn set_text(control: id, text: &str) {
    let text = ns(text);
    let _: () = msg_send![control,setStringValue:text];
    let _: () = msg_send![text, release];
}
extern "C" fn show(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.clear();
                s.startup.message.clear();
                s.startup.refresh();
                let app: id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app,activateIgnoringOtherApps:YES];
                let _: () = msg_send![s.startup.window,makeKeyAndOrderFront:nil];
            }
        }
    });
}
extern "C" fn toggle(_: &Object, _: Sel, sender: id) {
    STATE.with(|state|{if let Some(s)=state.borrow_mut().as_mut(){unsafe{
    let requested:isize=msg_send![sender,state];let before=status();let item=service();
    if item==nil{s.startup.message="macOS login registration is unavailable.".into();s.startup.refresh();return;}
    let enable=requested!=0;
    if enable && before==Status::Approval {
        s.startup.message="Approve LibreType in Login Items settings, or turn this option off.".into();s.startup.refresh();return;
    }
    if (enable && before==Status::Enabled) || (!enable && before==Status::Off){s.startup.message.clear();s.startup.refresh();return;}
    let mut error:id=nil;
    let success:BOOL=if enable{msg_send![item,registerAndReturnError:&mut error]}else{msg_send![item,unregisterAndReturnError:&mut error]};
    s.startup.message=if success==YES{String::new()}else if error!=nil{
        let code:isize=msg_send![error,code];format!("macOS could not change registration (error {code}). Check Login Items settings.")
    }else{"macOS did not confirm the change. Check Login Items settings.".into()};
    s.startup.refresh();
}}});
}
extern "C" fn settings(_: &Object, _: Sel, _: id) {
    unsafe {
        let _: () = msg_send![&SERVICE_CLASS, openSystemSettingsLoginItems];
    }
}

fn background_launch(event_class: u32, event_id: u32, reason: u32) -> bool {
    event_class == u32::from_be_bytes(*b"aevt")
        && event_id == u32::from_be_bytes(*b"oapp")
        && [u32::from_be_bytes(*b"lgit"), u32::from_be_bytes(*b"svit")].contains(&reason)
}
unsafe fn launched_in_background() -> bool {
    let manager: id = msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
    let event: id = msg_send![manager, currentAppleEvent];
    background_event(event)
}
unsafe fn background_event(event: id) -> bool {
    if event == nil {
        return false;
    }
    let event_class: u32 = msg_send![event, eventClass];
    let event_id: u32 = msg_send![event, eventID];
    let reason: id = msg_send![event,paramDescriptorForKeyword:u32::from_be_bytes(*b"prdt")];
    let reason: u32 = if reason == nil {
        0
    } else {
        msg_send![reason, enumCodeValue]
    };
    background_launch(event_class, event_id, reason)
}
extern "C" fn launched(this: &Object, _: Sel, _: id) {
    unsafe {
        if !launched_in_background() {
            on_setup(this, sel!(setup:), nil);
        }
    }
}
extern "C" fn reopened(this: &Object, _: Sel, _: id, _: BOOL) -> BOOL {
    on_setup(this, sel!(setup:), nil);
    NO
}
pub(super) unsafe fn register_actions(decl: &mut ClassDecl) {
    for (selector, action) in [
        (sel!(startup:), show as extern "C" fn(&Object, Sel, id)),
        (sel!(toggleLogin:), toggle),
        (sel!(loginSettings:), settings),
        (sel!(applicationDidFinishLaunching:), launched),
    ] {
        decl.add_method(selector, action);
    }
    decl.add_method(
        sel!(applicationShouldHandleReopen:hasVisibleWindows:),
        reopened as extern "C" fn(&Object, Sel, id, BOOL) -> BOOL,
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macos_approval_is_not_reported_as_enabled() {
        assert_eq!(Status::from_raw(0).checkbox(), 0);
        assert_eq!(Status::from_raw(1).checkbox(), 1);
        assert_eq!(Status::from_raw(2).checkbox(), -1);
        assert_eq!(Status::from_raw(3), Status::NotFound);
        assert_eq!(Status::from_raw(88), Status::Unknown);
    }
    #[test]
    fn synthetic_apple_event_parameters_are_read_without_sending_an_event() {
        unsafe {
            let pool: id = msg_send![class!(NSAutoreleasePool), new];
            assert!(!background_event(nil));
            let event: id = msg_send![class!(NSAppleEventDescriptor),appleEventWithEventClass:u32::from_be_bytes(*b"aevt") eventID:u32::from_be_bytes(*b"oapp") targetDescriptor:nil returnID:-1i16 transactionID:0i32];
            assert_ne!(event, nil);
            assert!(!background_event(event));
            let reason: id = msg_send![class!(NSAppleEventDescriptor),descriptorWithEnumCode:u32::from_be_bytes(*b"lgit")];
            let _: () =
                msg_send![event,setParamDescriptor:reason forKeyword:u32::from_be_bytes(*b"prdt")];
            assert!(background_event(event));
            let manual: id = msg_send![class!(NSAppleEventDescriptor),descriptorWithEnumCode:0u32];
            let _: () =
                msg_send![event,setParamDescriptor:manual forKeyword:u32::from_be_bytes(*b"prdt")];
            assert!(!background_event(event));
            let _: () = msg_send![pool, drain];
        }
    }
    #[test]
    fn only_login_and_service_open_events_suppress_settings() {
        let event = u32::from_be_bytes(*b"aevt");
        let open = u32::from_be_bytes(*b"oapp");
        let login = u32::from_be_bytes(*b"lgit");
        assert!(background_launch(event, open, login));
        assert!(background_launch(event, open, u32::from_be_bytes(*b"svit")));
        assert!(!background_launch(event, open, 0));
        assert!(!background_launch(
            event,
            u32::from_be_bytes(*b"rapp"),
            login
        ));
        assert!(!background_launch(0, open, login));
    }
}
