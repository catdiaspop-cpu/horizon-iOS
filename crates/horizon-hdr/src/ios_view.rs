//! iOS: what the game draws into, in UIKit's own numbers.
//!
//! A frame reaches the screen through winit's `UIView` and the `CAMetalLayer` wgpu made for it,
//! and it fills the screen only when those three agree: a view that does not fill its window, or
//! a layer that is not the view's size, leaves the picture off-centre with the rest of the
//! display black - which on a device with no debugger looks exactly like a renderer bug. Reading
//! the numbers back at startup settles which of them it is.

use std::ffi::c_void;

use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
// `NSRect` is UIKit's `CGRect` under the name objc2-foundation 0.3 exports at its root.
use objc2_foundation::NSRect;

/// A line for the log: the screen, the window, the view and its layer.
pub fn describe(view: *mut c_void) -> String {
    // SAFETY: `view` is the `UIView` of the window the caller owns and keeps alive for this call,
    // and every message below only reads a value from it, from its window, from its layer or from
    // the screen.
    unsafe {
        let view = &*(view as *const AnyObject);
        let window: *mut AnyObject = msg_send![view, window];
        let layer: *mut AnyObject = msg_send![view, layer];
        let view_bounds: NSRect = msg_send![view, bounds];
        let view_frame: NSRect = msg_send![view, frame];
        let view_scale: f64 = msg_send![view, contentScaleFactor];
        let window_bounds: Option<NSRect> = if window.is_null() {
            None
        } else {
            Some(msg_send![window, bounds])
        };
        let layer_frame: Option<NSRect> = if layer.is_null() {
            None
        } else {
            Some(msg_send![layer, frame])
        };
        let layer_scale: Option<f64> = if layer.is_null() {
            None
        } else {
            Some(msg_send![layer, contentsScale])
        };
        let screen: *mut AnyObject = match AnyClass::get(c"UIScreen") {
            Some(class) => msg_send![class, mainScreen],
            None => std::ptr::null_mut(),
        };
        let screen_bounds: Option<NSRect> = if screen.is_null() {
            None
        } else {
            Some(msg_send![screen, bounds])
        };
        let screen_scale: Option<f64> = if screen.is_null() {
            None
        } else {
            Some(msg_send![screen, scale])
        };
        format!(
            "screen {} at scale {}; window {}; view frame {}, bounds {} at scale {:.1}; \
             layer frame {} at scale {}",
            points(screen_bounds),
            scale(screen_scale),
            points(window_bounds),
            points(Some(view_frame)),
            points(Some(view_bounds)),
            view_scale,
            points(layer_frame),
            scale(layer_scale),
        )
    }
}

/// A rect in points, as UIKit prints it.
fn points(rect: Option<NSRect>) -> String {
    match rect {
        Some(rect) => format!(
            "{:.0}x{:.0} at ({:.0}, {:.0}) pt",
            rect.size.width, rect.size.height, rect.origin.x, rect.origin.y
        ),
        None => "none".to_string(),
    }
}

/// A scale factor, or `none` when the object that has it is missing.
fn scale(scale: Option<f64>) -> String {
    match scale {
        Some(scale) => format!("{scale:.1}"),
        None => "none".to_string(),
    }
}
