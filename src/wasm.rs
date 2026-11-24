// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The WebAssembly boundary: text in, text out, through the module's memory. No generated glue;
//! `js/index.ts` is the other side.
//!
//! To call a function: `alloc` room for the UTF-8 request, write it, call the function with the
//! pointer and length. It returns the answer's pointer in the upper and its length in the lower
//! 32 bits. Read the answer, then `dealloc` both request and answer.

use std::alloc::{Layout, alloc as allocate, dealloc as deallocate};

fn layout(len: usize) -> Layout {
    Layout::from_size_align(len.max(1), 1).expect("a byte buffer has a valid layout")
}

#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    unsafe { allocate(layout(len)) }
}

/// # Safety
/// `ptr` and `len` must come from [`alloc`] or from an answer of this module.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    unsafe { deallocate(ptr, layout(len)) }
}

fn answer(text: String) -> u64 {
    let bytes = text.into_bytes().into_boxed_slice();
    let len = bytes.len();
    let ptr = if len == 0 { alloc(0) } else { Box::into_raw(bytes) as *mut u8 };
    ((ptr as u64) << 32) | len as u64
}

/// # Safety
/// `ptr` and `len` must describe a buffer from [`alloc`] holding the request.
unsafe fn ask(ptr: *const u8, len: usize, question: fn(&str) -> String) -> u64 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    answer(match std::str::from_utf8(input) {
        Ok(text) => question(text),
        Err(_) => r#"{"error":"the request is not UTF-8"}"#.to_string(),
    })
}

/// # Safety
/// `ptr` and `len` must describe a buffer from [`alloc`] holding the request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn snap(ptr: *const u8, len: usize) -> u64 {
    unsafe { ask(ptr, len, crate::api::snap_json) }
}

/// # Safety
/// `ptr` and `len` must describe a buffer from [`alloc`] holding the request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn route(ptr: *const u8, len: usize) -> u64 {
    unsafe { ask(ptr, len, crate::api::route_json) }
}

/// # Safety
/// `ptr` and `len` must describe a buffer from [`alloc`] holding the request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arrange(ptr: *const u8, len: usize) -> u64 {
    unsafe { ask(ptr, len, crate::api::arrange_json) }
}
