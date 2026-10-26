//! Carbon hotkeys go directly to the event dispatcher used by AppKit's loop.
//! No keyboard event tap, polling, or additional privacy permission.
use anyhow::{Result, ensure};
use std::{collections::HashMap, ffi::c_void};
type Ref = *mut c_void;
#[repr(C)]
struct EventType {
    class: u32,
    kind: u32,
}
#[repr(C)]
struct HotkeyId {
    signature: u32,
    id: u32,
}
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetEventDispatcherTarget() -> Ref;
    fn InstallEventHandler(
        target: Ref,
        handler: unsafe extern "C" fn(Ref, Ref, Ref) -> i32,
        count: u32,
        types: *const EventType,
        data: Ref,
        out: *mut Ref,
    ) -> i32;
    fn RemoveEventHandler(handler: Ref) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: HotkeyId,
        target: Ref,
        options: u32,
        out: *mut Ref,
    ) -> i32;
    fn UnregisterEventHotKey(hotkey: Ref) -> i32;
    fn GetEventParameter(
        event: Ref,
        name: u32,
        kind: u32,
        actual_type: *mut u32,
        size: u32,
        actual_size: *mut u32,
        data: Ref,
    ) -> i32;
}
pub struct NativeHotkeyManager {
    handler: Ref,
    callback: Box<Box<dyn Fn(u32)>>,
    keys: HashMap<u32, Ref>,
}
impl NativeHotkeyManager {
    pub fn new(callback: impl Fn(u32) + 'static) -> Result<Self> {
        ensure!(
            objc2::MainThreadMarker::new().is_some(),
            "热键必须在主线程注册"
        );
        let callback = Box::new(Box::new(callback) as Box<dyn Fn(u32)>);
        let data = (&*callback as *const Box<dyn Fn(u32)>).cast_mut().cast();
        let mut handler = std::ptr::null_mut();
        let event = EventType {
            class: u32::from_be_bytes(*b"keyb"),
            kind: 5,
        };
        let result = unsafe {
            InstallEventHandler(
                GetEventDispatcherTarget(),
                pressed,
                1,
                &event,
                data,
                &mut handler,
            )
        };
        ensure!(result == 0, "系统热键分发器初始化失败 ({result})");
        Ok(Self {
            handler,
            callback,
            keys: HashMap::new(),
        })
    }
    pub fn register(&mut self, code: u32, modifiers: u32, id: u32) -> Result<()> {
        let mut reference = std::ptr::null_mut();
        // kEventHotKeyExclusive=1 reports actual conflicts, rather than silently
        // sharing a binding with another application that consumes it first.
        let result = unsafe {
            RegisterEventHotKey(
                code,
                modifiers,
                HotkeyId {
                    signature: u32::from_be_bytes(*b"ssnp"),
                    id,
                },
                GetEventDispatcherTarget(),
                1,
                &mut reference,
            )
        };
        ensure!(
            result == 0,
            "系统拒绝注册快捷键 ({result})，可能与其他应用或系统快捷键冲突"
        );
        self.keys.insert(id, reference);
        Ok(())
    }
    pub fn unregister(&mut self, id: u32) -> Result<()> {
        if let Some(reference) = self.keys.remove(&id) {
            let result = unsafe { UnregisterEventHotKey(reference) };
            ensure!(result == 0, "系统注销热键失败 ({result})");
        }
        Ok(())
    }
}
impl Drop for NativeHotkeyManager {
    fn drop(&mut self) {
        for (_, key) in self.keys.drain() {
            unsafe {
                UnregisterEventHotKey(key);
            }
        }
        unsafe {
            RemoveEventHandler(self.handler);
        }
        let _ = &self.callback; // Must outlive removal of its native handler.
    }
}
unsafe extern "C" fn pressed(_: Ref, event: Ref, data: Ref) -> i32 {
    let mut id = HotkeyId {
        signature: 0,
        id: 0,
    };
    let result = unsafe {
        GetEventParameter(
            event,
            u32::from_be_bytes(*b"----"),
            u32::from_be_bytes(*b"hkid"),
            std::ptr::null_mut(),
            std::mem::size_of::<HotkeyId>() as u32,
            std::ptr::null_mut(),
            (&mut id as *mut HotkeyId).cast(),
        )
    };
    if result == 0 && id.signature == u32::from_be_bytes(*b"ssnp") && !data.is_null() {
        let callback = unsafe { &*data.cast::<Box<dyn Fn(u32)>>() };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(id.id)));
        0
    } else {
        -9874
    } // eventNotHandledErr: allow unrelated native events through.
}
