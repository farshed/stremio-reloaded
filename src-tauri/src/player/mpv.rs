use std::ffi::{c_char, c_int, CStr, CString};
use std::fmt;
use std::ptr;

use libmpv2_sys as sys;
use serde_json::{Map, Number, Value};

#[derive(Debug, Clone, Copy)]
pub struct MpvError(pub c_int);

impl fmt::Display for MpvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&error_string(self.0))
    }
}

impl std::error::Error for MpvError {}

pub fn error_string(code: c_int) -> String {
    let text = unsafe { sys::mpv_error_string(code) };
    if text.is_null() {
        return format!("mpv error {code}");
    }
    unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned()
}

fn check(code: c_int) -> Result<(), MpvError> {
    if code < 0 {
        Err(MpvError(code))
    } else {
        Ok(())
    }
}

fn c_string(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

pub struct Mpv {
    handle: *mut sys::mpv_handle,
}

unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

pub enum Event {
    None,
    Shutdown,
    PropertyChange {
        id: u64,
        name: String,
        data: Option<Value>,
        error: Option<String>,
    },
    EndFile {
        id: u64,
        reason: &'static str,
        error: Option<String>,
    },
    Other,
}

impl Mpv {
    pub fn new(options: &[(&str, &str)]) -> Result<Self, MpvError> {
        unsafe { libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr()) };
        let handle = unsafe { sys::mpv_create() };
        if handle.is_null() {
            return Err(MpvError(sys::mpv_error_MPV_ERROR_NOMEM));
        }
        let mpv = Self { handle };
        for (name, value) in options {
            if let Err(err) = mpv.set_option(name, value) {
                log::warn!("mpv option {name}={value} rejected: {err}");
            }
        }
        check(unsafe { sys::mpv_initialize(handle) })?;
        Ok(mpv)
    }

    pub fn raw(&self) -> *mut sys::mpv_handle {
        self.handle
    }

    pub fn set_option(&self, name: &str, value: &str) -> Result<(), MpvError> {
        let (name, value) = (c_string(name), c_string(value));
        check(unsafe { sys::mpv_set_option_string(self.handle, name.as_ptr(), value.as_ptr()) })
    }

    pub fn command(&self, args: &Value) -> Result<(), MpvError> {
        let mut builder = NodeBuilder::default();
        let mut node = builder.build(args);
        let mut result = empty_node();
        let code = unsafe { sys::mpv_command_node(self.handle, &mut node, &mut result) };
        unsafe { sys::mpv_free_node_contents(&mut result) };
        check(code)
    }

    pub fn set_property(&self, name: &str, value: &Value) -> Result<(), MpvError> {
        let name = c_string(name);
        let mut builder = NodeBuilder::default();
        let mut node = builder.build(value);
        check(unsafe {
            sys::mpv_set_property(
                self.handle,
                name.as_ptr(),
                sys::mpv_format_MPV_FORMAT_NODE,
                (&mut node as *mut sys::mpv_node).cast(),
            )
        })
    }

    pub fn get_property(&self, name: &str) -> Option<Value> {
        let name = c_string(name);
        let mut node = empty_node();
        let code = unsafe {
            sys::mpv_get_property(
                self.handle,
                name.as_ptr(),
                sys::mpv_format_MPV_FORMAT_NODE,
                (&mut node as *mut sys::mpv_node).cast(),
            )
        };
        if code < 0 {
            return None;
        }
        let value = unsafe { node_to_json(&node) };
        unsafe { sys::mpv_free_node_contents(&mut node) };
        Some(value)
    }

    pub fn observe_property(&self, name: &str) -> Result<(), MpvError> {
        let name = c_string(name);
        check(unsafe { sys::mpv_observe_property(self.handle, 0, name.as_ptr(), sys::mpv_format_MPV_FORMAT_NODE) })
    }

    pub fn wait_event(&self, timeout: f64) -> Event {
        let event = unsafe { &*sys::mpv_wait_event(self.handle, timeout) };
        let error = (event.error < 0).then(|| error_string(event.error));
        let id = event.reply_userdata;
        match event.event_id {
            sys::mpv_event_id_MPV_EVENT_NONE => Event::None,
            sys::mpv_event_id_MPV_EVENT_SHUTDOWN => Event::Shutdown,
            sys::mpv_event_id_MPV_EVENT_PROPERTY_CHANGE => {
                let prop = unsafe { &*(event.data as *const sys::mpv_event_property) };
                let name = unsafe { CStr::from_ptr(prop.name) }.to_string_lossy().into_owned();
                let data = unsafe { property_data(prop) };
                Event::PropertyChange { id, name, data, error }
            }
            sys::mpv_event_id_MPV_EVENT_END_FILE => {
                let end = unsafe { &*(event.data as *const sys::mpv_event_end_file) };
                let (reason, error) = match end.reason {
                    sys::mpv_end_file_reason_MPV_END_FILE_REASON_ERROR => ("error", Some(error_string(end.error))),
                    sys::mpv_end_file_reason_MPV_END_FILE_REASON_QUIT => ("quit", error),
                    _ => ("other", error),
                };
                Event::EndFile { id, reason, error }
            }
            _ => Event::Other,
        }
    }
}

fn empty_node() -> sys::mpv_node {
    sys::mpv_node {
        u: sys::mpv_node__bindgen_ty_1 { int64: 0 },
        format: sys::mpv_format_MPV_FORMAT_NONE,
    }
}

unsafe fn property_data(prop: &sys::mpv_event_property) -> Option<Value> {
    if prop.data.is_null() {
        return None;
    }
    match prop.format {
        sys::mpv_format_MPV_FORMAT_NODE => Some(node_to_json(&*(prop.data as *const sys::mpv_node))),
        sys::mpv_format_MPV_FORMAT_DOUBLE => Number::from_f64(*(prop.data as *const f64)).map(Value::Number),
        sys::mpv_format_MPV_FORMAT_FLAG => Some(Value::from(*(prop.data as *const c_int))),
        sys::mpv_format_MPV_FORMAT_STRING => {
            let text = *(prop.data as *const *const c_char);
            Some(Value::String(CStr::from_ptr(text).to_string_lossy().into_owned()))
        }
        _ => None,
    }
}

unsafe fn node_to_json(node: &sys::mpv_node) -> Value {
    match node.format {
        sys::mpv_format_MPV_FORMAT_STRING => {
            Value::String(CStr::from_ptr(node.u.string).to_string_lossy().into_owned())
        }
        sys::mpv_format_MPV_FORMAT_FLAG => Value::Bool(node.u.flag != 0),
        sys::mpv_format_MPV_FORMAT_INT64 => Value::from(node.u.int64),
        sys::mpv_format_MPV_FORMAT_DOUBLE => Number::from_f64(node.u.double_).map_or(Value::Null, Value::Number),
        sys::mpv_format_MPV_FORMAT_NODE_ARRAY => {
            let list = &*node.u.list;
            let values = list_values(list);
            Value::Array(values.iter().map(|v| node_to_json(v)).collect())
        }
        sys::mpv_format_MPV_FORMAT_NODE_MAP => {
            let list = &*node.u.list;
            let values = list_values(list);
            let keys: &[*mut c_char] = if list.keys.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(list.keys, values.len())
            };
            let map: Map<String, Value> = keys
                .iter()
                .zip(values)
                .map(|(k, v)| (CStr::from_ptr(*k).to_string_lossy().into_owned(), node_to_json(v)))
                .collect();
            Value::Object(map)
        }
        _ => Value::Null,
    }
}

unsafe fn list_values(list: &sys::mpv_node_list) -> &[sys::mpv_node] {
    if list.values.is_null() || list.num <= 0 {
        &[]
    } else {
        std::slice::from_raw_parts(list.values, list.num as usize)
    }
}

#[derive(Default)]
struct NodeBuilder {
    strings: Vec<CString>,
    // Boxed so list addresses stay fixed while the Vec grows.
    #[allow(clippy::vec_box)]
    lists: Vec<Box<sys::mpv_node_list>>,
    values: Vec<Vec<sys::mpv_node>>,
    keys: Vec<Vec<*mut c_char>>,
}

impl NodeBuilder {
    fn build(&mut self, value: &Value) -> sys::mpv_node {
        let mut node = empty_node();
        match value {
            Value::Null => {}
            Value::Bool(b) => {
                node.format = sys::mpv_format_MPV_FORMAT_FLAG;
                node.u.flag = c_int::from(*b);
            }
            Value::Number(n) => match n.as_i64() {
                Some(i) => {
                    node.format = sys::mpv_format_MPV_FORMAT_INT64;
                    node.u.int64 = i;
                }
                None => {
                    node.format = sys::mpv_format_MPV_FORMAT_DOUBLE;
                    node.u.double_ = n.as_f64().unwrap_or_default();
                }
            },
            Value::String(s) => {
                node.format = sys::mpv_format_MPV_FORMAT_STRING;
                node.u.string = self.string(s);
            }
            Value::Array(items) => {
                node.format = sys::mpv_format_MPV_FORMAT_NODE_ARRAY;
                let values: Vec<sys::mpv_node> = items.iter().map(|v| self.build(v)).collect();
                node.u.list = self.list(values, ptr::null_mut());
            }
            Value::Object(map) => {
                node.format = sys::mpv_format_MPV_FORMAT_NODE_MAP;
                let values: Vec<sys::mpv_node> = map.values().map(|v| self.build(v)).collect();
                let mut keys: Vec<*mut c_char> = map.keys().map(|k| self.string(k)).collect();
                let keys_ptr = keys.as_mut_ptr();
                self.keys.push(keys);
                node.u.list = self.list(values, keys_ptr);
            }
        }
        node
    }

    fn string(&mut self, text: &str) -> *mut c_char {
        let owned = c_string(text);
        let ptr = owned.as_ptr() as *mut c_char;
        self.strings.push(owned);
        ptr
    }

    fn list(&mut self, mut values: Vec<sys::mpv_node>, keys: *mut *mut c_char) -> *mut sys::mpv_node_list {
        let mut list = Box::new(sys::mpv_node_list {
            num: values.len() as c_int,
            values: values.as_mut_ptr(),
            keys,
        });
        let ptr: *mut sys::mpv_node_list = &mut *list;
        self.values.push(values);
        self.lists.push(list);
        ptr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEST_PATTERN: &str = "av://lavfi:testsrc=duration=1:size=320x240:rate=25";

    #[test]
    fn plays_a_file_and_reports_events() {
        let mpv = Mpv::new(&[("vo", "null"), ("ao", "null")]).expect("mpv starts");
        for name in ["vid", "duration", "time-pos", "pause", "path"] {
            mpv.observe_property(name).expect("property can be observed");
        }
        mpv.command(&json!(["loadfile", TEST_PATTERN])).expect("loadfile accepted");
        mpv.set_property("pause", &json!(false)).expect("pause can be set");

        let (mut vid, mut duration, mut time_positions, mut ended) = (None, None, 0, None);
        for _ in 0..500 {
            match mpv.wait_event(5.0) {
                Event::PropertyChange { name, data: Some(data), .. } => match name.as_str() {
                    "vid" => vid = Some(data),
                    "duration" => duration = data.as_f64(),
                    "time-pos" => time_positions += 1,
                    _ => {}
                },
                Event::EndFile { reason, error, .. } => {
                    ended = Some((reason, error));
                    break;
                }
                _ => {}
            }
        }

        assert_eq!(vid, Some(json!(1)));
        assert!(duration.is_some_and(|d| (d - 1.0).abs() < 0.1), "duration was {duration:?}");
        assert!(time_positions > 5, "only {time_positions} time-pos updates");
        assert_eq!(ended, Some(("other", None)));
        assert!(mpv.command(&json!(["no-such-command"])).is_err());
    }

    #[test]
    fn converts_nested_values_both_ways() {
        let mpv = Mpv::new(&[("vo", "null"), ("ao", "null")]).expect("mpv starts");
        let value = json!({ "a": [1, 2.5, "x", true, null], "b": { "c": "d" } });
        mpv.set_property("user-data/test", &value).expect("nested value accepted");
        assert_eq!(mpv.get_property("user-data/test"), Some(value));
    }
}
