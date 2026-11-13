//! iOS: what the game draws into, in UIKit's own numbers.
//!
//! A frame reaches the screen through winit's `UIView` and the `CAMetalLayer` wgpu made for it,
//! and it fills the screen only when those three agree: a view that does not fill its window, or
//! a layer that is not the view's size, leaves the picture off-centre with the rest of the
//! display black - which on a device with no debugger looks exactly like a renderer bug. Reading
//! the numbers back at startup settles which of them it is.

use std::ffi::c_void;

use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;
use objc2_foundation::CGRect;

/// A line for the log: the screen, the window, the view and its layer.
pub fn describe(view: *mut c_void) -> String {
    // SAFETY: `view` is the `UIView` of the window the caller owns and keeps alive for this call,
    // and every message below only reads a value from it, from its window, from its layer or from
    // the screen.
    unsafe {
        let view = &*(view as *const AnyObject);
        let window: *mut AnyObject = msg_send![view, window];
        let layer: *mut AnyObject = msg_send![view, layer];
        let view_bounds: CGRect = msg_send![view, bounds];
        let view_frame: CGRect = msg_send![view, frame];
        let view_scale: f64 = msg_send![view, contentScaleFactor];
        let (window_bounds, layer_frame, layer_scale) = if window.is_null() || layer.is_null() {
            (CGRect::ZERO, CGRect::ZERO, 0.0)
        } else {
            (
                msg_send![window, bounds],
                msg_send![layer, frame],
                msg_send![layer, contentsScale],
            )
        };
        let screen = AnyClass::get(c"UIScreen").map(|class| msg_send![class, mainScreen]);
        let (screen_bounds, screen_scale) = match screen {
            Some(screen) if !screen.is_null() => {
                (msg_send![screen, bounds], msg_send![screen, scale])
            }
            _ => (CGRect::ZERO, 0.0),
        };
        format!(
            "screen {}, at scale {:.1}; window {}; view frame {}, bounds {} at scale {:.1}; \
             layer frame {} at scale {:.1}",
            pt(screen_bounds),
            screen_scale,
            pt(window_bounds),
            pt(view_frame),
            pt(view_bounds),
            view_scale,
            pt(layer_frame),
            layer_scale,
        )
    }
}

/// A rect in points, as UIKit prints it.
fn pt(rect: CGRect) -> String {
    format!(
        "{:.0}x{:.0} at ({:.0}, {:.0}) pt",
        rect.size.width, rect.size.height, rect.origin.x, rect.origin.y
    )
}
