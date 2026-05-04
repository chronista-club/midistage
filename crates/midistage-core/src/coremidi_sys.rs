//! CoreMIDI C API バインディング（最小限、手書き）
//!
//! macOS の CoreMIDI.framework から必要な関数のみ宣言する。
//! MIDI 2.0 (UMP) 入力に必要な API のみ。
//!
//! ## Vendored origin
//!
//! Vendored from `cplp-sound-system/crates/cplp-midi/src/coremidi_sys.rs`
//! (snapshot 2026-05-04)。 RX-only な FFI bindings で、
//! TX 側 (`MIDIOutputPortCreate` / `MIDISendEventList` / `MIDIDestinationCreateWithProtocol` 等)
//! は v0-alpha で追加する。 v1+ で α 正規化 (cplp-sound-system 側を本 crate 依存に切替) を計画。

// CoreMIDI の定数名・フィールド名はそのまま保持するため lint を抑制

use std::ffi::c_void;

// ── 基本型 ──────────────────────────────────────

pub type MIDIClientRef = u32;
pub type MIDIPortRef = u32;
pub type MIDIEndpointRef = u32;
pub type MIDIObjectRef = u32;
pub type OSStatus = i32;
pub type MIDIProtocolID = i32;
pub type MIDITimeStamp = u64;

pub const noErr: OSStatus = 0;
pub const kMIDIProtocol_2_0: MIDIProtocolID = 2;

// kMIDIPropertyDisplayName — CoreMIDI の定数文字列
// Core Foundation の CFStringRef だが、Rust では直接参照する
unsafe extern "C" {
    pub static kMIDIPropertyDisplayName: *const c_void; // CFStringRef
}

// ── MIDIEventPacket / MIDIEventList ─────────────

/// MIDIEventPacket — UMP パケット（最大 64 words）
///
/// CoreMIDI の MIDIEventPacket は可変長だが、words は最大 64 個。
#[repr(C)]
pub struct MIDIEventPacket {
    pub timeStamp: MIDITimeStamp,
    pub wordCount: u32,
    pub words: [u32; 64],
}

/// MIDIEventList — パケットの配列
///
/// CoreMIDI の MIDIEventList は可変長。`packet` は最初のパケットへのポインタ。
/// 後続パケットは MIDIEventPacketNext() でイテレートする。
#[repr(C)]
pub struct MIDIEventList {
    pub protocol: MIDIProtocolID,
    pub numPackets: u32,
    pub packet: [MIDIEventPacket; 1], // flexible array member
}

// ── MIDINotification ─────────────────────────────

#[repr(C)]
pub struct MIDINotification {
    pub messageID: i32,
    pub messageSize: u32,
}

pub const kMIDIMsgSetupChanged: i32 = 1;

// ── コールバック型 ─────────────────────────────

/// MIDIReceiveBlock — UMP パケット受信コールバック
///
/// `MIDIInputPortCreateWithProtocol` の readBlock パラメータ。
/// CoreMIDI は Objective-C Block として渡すことを期待するが、
/// Rust からは `extern "C" fn` + ユーザーデータで代替する。
pub type MIDIReadBlock = *const c_void; // Block_literal pointer

/// MIDINotifyBlock
pub type MIDINotifyBlock = *const c_void;

// ── CoreMIDI 関数 ─────────────────────────────

#[link(name = "CoreMIDI", kind = "framework")]
unsafe extern "C" {
    pub fn MIDIClientCreateWithBlock(
        name: *const c_void, // CFStringRef
        outClient: *mut MIDIClientRef,
        notifyBlock: MIDINotifyBlock,
    ) -> OSStatus;

    pub fn MIDIInputPortCreateWithProtocol(
        client: MIDIClientRef,
        portName: *const c_void, // CFStringRef
        protocol: MIDIProtocolID,
        outPort: *mut MIDIPortRef,
        readBlock: MIDIReadBlock,
    ) -> OSStatus;

    pub fn MIDIPortConnectSource(
        port: MIDIPortRef,
        source: MIDIEndpointRef,
        connRefCon: *mut c_void,
    ) -> OSStatus;

    pub fn MIDIPortDispose(port: MIDIPortRef) -> OSStatus;
    pub fn MIDIClientDispose(client: MIDIClientRef) -> OSStatus;

    pub fn MIDIGetNumberOfSources() -> u32;
    pub fn MIDIGetSource(sourceIndex0: u32) -> MIDIEndpointRef;

    pub fn MIDIObjectGetStringProperty(
        obj: MIDIObjectRef,
        propertyID: *const c_void, // CFStringRef
        str_: *mut *const c_void,  // CFStringRef *
    ) -> OSStatus;

    /// 次のパケットへのポインタを返す
    pub fn MIDIEventPacketNext(pkt: *const MIDIEventPacket) -> *const MIDIEventPacket;
}

// ── Core Foundation ヘルパー ─────────────────

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFStringCreateWithCString(
        alloc: *const c_void,
        cStr: *const u8,
        encoding: u32,
    ) -> *const c_void; // CFStringRef

    pub fn CFRelease(cf: *const c_void);

    pub fn CFStringGetCStringPtr(theString: *const c_void, encoding: u32) -> *const u8;

    pub fn CFStringGetCString(
        theString: *const c_void,
        buffer: *mut u8,
        bufferSize: i64,
        encoding: u32,
    ) -> bool;
}

/// kCFStringEncodingUTF8
pub const kCFStringEncodingUTF8: u32 = 0x0800_0100;

/// CFString を作成するヘルパー
///
/// # Safety
/// 返された CFStringRef は使用後に `CFRelease` すること。
pub unsafe fn cfstring_from_str(s: &str) -> *const c_void {
    let cstr = std::ffi::CString::new(s).unwrap();
    unsafe {
        CFStringCreateWithCString(
            std::ptr::null(),
            cstr.as_ptr() as *const u8,
            kCFStringEncodingUTF8,
        )
    }
}

/// CFStringRef → String に変換
///
/// # Safety
/// `cf_str` は有効な CFStringRef であること。
pub unsafe fn cfstring_to_string(cf_str: *const c_void) -> Option<String> {
    if cf_str.is_null() {
        return None;
    }

    // 高速パス: C 文字列ポインタを直接取得
    let ptr = unsafe { CFStringGetCStringPtr(cf_str, kCFStringEncodingUTF8) };
    if !ptr.is_null() {
        let cstr = unsafe { std::ffi::CStr::from_ptr(ptr as *const i8) };
        return cstr.to_str().ok().map(|s| s.to_string());
    }

    // 遅いパス: バッファにコピー
    let mut buf = [0u8; 256];
    let ok = unsafe {
        CFStringGetCString(
            cf_str,
            buf.as_mut_ptr(),
            buf.len() as i64,
            kCFStringEncodingUTF8,
        )
    };
    if ok {
        let cstr = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr() as *const i8) };
        cstr.to_str().ok().map(|s| s.to_string())
    } else {
        None
    }
}
