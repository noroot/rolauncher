// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2025-2026 dmth

//! Saves PNG captures of our own windows. A process may always capture its own
//! windows, so this needs no Screen Recording permission. Used by `--screenshots <dir>`.

use std::ffi::c_void;
use std::path::Path;

use objc2_app_kit::NSWindow;
use objc2_foundation::{ns_string, NSString, NSURL};

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

const CG_RECT_NULL: CGRect = CGRect { x: f64::INFINITY, y: f64::INFINITY, w: 0.0, h: 0.0 };
const K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW: u32 = 1 << 3;
const K_CG_WINDOW_IMAGE_BEST_RESOLUTION: u32 = 1 << 3;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCreateImage(bounds: CGRect, list_option: u32, window_id: u32, image_option: u32) -> *mut c_void;
    fn CGImageRelease(image: *mut c_void);
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageDestinationCreateWithURL(url: *const c_void, ty: *const c_void, count: usize, options: *const c_void) -> *mut c_void;
    fn CGImageDestinationAddImage(dest: *mut c_void, image: *mut c_void, properties: *const c_void);
    fn CGImageDestinationFinalize(dest: *mut c_void) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *mut c_void);
}

/// Writes `window` (including its shadow) as a PNG to `path`.
pub fn capture(window: &NSWindow, path: &Path) -> Result<(), String> {
    let id = u32::try_from(window.windowNumber()).map_err(|_| "window has no number")?;
    // NSURL and NSString are toll-free bridged with CFURL / CFString.
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let png_type = ns_string!("public.png");
    unsafe {
        let image = CGWindowListCreateImage(CG_RECT_NULL, K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW, id, K_CG_WINDOW_IMAGE_BEST_RESOLUTION);
        if image.is_null() {
            return Err(format!("could not capture window {id}"));
        }
        let dest = CGImageDestinationCreateWithURL(
            (&*url as *const NSURL).cast(),
            (png_type as *const NSString).cast(),
            1,
            std::ptr::null(),
        );
        if dest.is_null() {
            CGImageRelease(image);
            return Err(format!("could not create {}", path.display()));
        }
        CGImageDestinationAddImage(dest, image, std::ptr::null());
        let ok = CGImageDestinationFinalize(dest);
        CFRelease(dest);
        CGImageRelease(image);
        if ok { Ok(()) } else { Err(format!("could not write {}", path.display())) }
    }
}
