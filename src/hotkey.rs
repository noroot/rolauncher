// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2025-2026 dmth

//! Global hotkey via Carbon `RegisterEventHotKey` (works without Accessibility permission).

use std::ffi::c_void;

type OSStatus = i32;
type EventTargetRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventHandlerCallRef = *mut c_void;
type EventRef = *mut c_void;
type EventHotKeyRef = *mut c_void;
type EventHandlerUPP =
    extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus;

#[repr(C)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> OSStatus;
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: EventHandlerUPP,
        num_types: usize,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> OSStatus;
}

const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;

pub const KEY_SPACE: u32 = 49;
pub const MOD_CMD: u32 = 1 << 8;

thread_local! {
    static CALLBACK: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

extern "C" fn on_hotkey(_: EventHandlerCallRef, _: EventRef, _: *mut c_void) -> OSStatus {
    // Carbon dispatches application events on the main thread.
    CALLBACK.with(|cb| {
        if let Some(cb) = cb.borrow().as_ref() {
            cb();
        }
    });
    0
}

/// Registers a single process-wide hotkey. Must be called on the main thread.
pub fn register(key_code: u32, modifiers: u32, callback: impl Fn() + 'static) -> Result<(), OSStatus> {
    CALLBACK.with(|cb| *cb.borrow_mut() = Some(Box::new(callback)));
    unsafe {
        let target = GetApplicationEventTarget();
        let spec = EventTypeSpec { event_class: K_EVENT_CLASS_KEYBOARD, event_kind: K_EVENT_HOT_KEY_PRESSED };
        let status = InstallEventHandler(target, on_hotkey, 1, &spec, std::ptr::null_mut(), std::ptr::null_mut());
        if status != 0 {
            return Err(status);
        }
        let id = EventHotKeyID { signature: u32::from_be_bytes(*b"HKLR"), id: 1 };
        let mut hotkey_ref: EventHotKeyRef = std::ptr::null_mut();
        let status = RegisterEventHotKey(key_code, modifiers, id, target, 0, &mut hotkey_ref);
        if status != 0 {
            return Err(status);
        }
    }
    Ok(())
}
