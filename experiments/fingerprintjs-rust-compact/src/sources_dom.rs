//! Layout-based FingerprintJS v5.2.0 entropy sources.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use mires_js::{callback0, callback1, callback2, invoke, JsValue};

use crate::blocker_data::BLOCKER_FILTERS;
use crate::host_dom::{
    add_style_string, append_child, create_element, dom_is_android, dom_is_chromium,
    dom_is_chromium_128_or_newer, dom_is_gecko, dom_is_gecko_143_or_newer, dom_is_safari_webkit,
    dom_is_webkit, dom_is_webkit_616_or_newer, exit_fullscreen_dom, frame_array,
    fullscreen_element, host_string, is_nullish, natural_font_text, remove_from_parent,
    resolved_promise, set_style_property, wait_for_document_body, with_iframe,
};
use crate::{
    array, as_promise, call, document, get, global_this, join_as_strings, number_value, object,
    parse_float, push, rejected_promise, round, screen, set, set_index, sort, string_value,
};

const TEST_STRING: &str = "mmMwWLliI0O&1";
const TEXT_SIZE: &str = "48px";
const BASE_FONTS: [&str; 3] = ["monospace", "sans-serif", "serif"];
const FONT_LIST: [&str; 52] = [
    "sans-serif-thin",
    "ARNO PRO",
    "Agency FB",
    "Arabic Typesetting",
    "Arial Unicode MS",
    "AvantGarde Bk BT",
    "BankGothic Md BT",
    "Batang",
    "Bitstream Vera Sans Mono",
    "Calibri",
    "Century",
    "Century Gothic",
    "Clarendon",
    "EUROSTILE",
    "Franklin Gothic",
    "Futura Bk BT",
    "Futura Md BT",
    "GOTHAM",
    "Gill Sans",
    "HELV",
    "Haettenschweiler",
    "Helvetica Neue",
    "Humanst521 BT",
    "Leelawadee",
    "Letter Gothic",
    "Levenim MT",
    "Lucida Bright",
    "Lucida Sans",
    "Menlo",
    "MS Mincho",
    "MS Outlook",
    "MS Reference Specialty",
    "MS UI Gothic",
    "MT Extra",
    "MYRIAD PRO",
    "Marlett",
    "Meiryo UI",
    "Microsoft Uighur",
    "Minion Pro",
    "Monotype Corsiva",
    "PMingLiU",
    "Pristina",
    "SCRIPTINA",
    "Segoe UI Light",
    "Serifa",
    "SimHei",
    "Small Fonts",
    "Staccato222 BT",
    "TRAJAN PRO",
    "Univers CE 55 Medium",
    "Vrinda",
    "ZWAdobeF",
];

const FONT_PREFERENCE_TEXT: &str = "mmMwWLliI0fiflO&1";
const FONT_PREFERENCES: [(&str, &str, &str); 7] = [
    ("default", "", ""),
    ("apple", "font", "-apple-system-body"),
    ("serif", "fontFamily", "serif"),
    ("sans", "fontFamily", "sans-serif"),
    ("mono", "fontFamily", "monospace"),
    ("min", "fontSize", "1px"),
    ("system", "fontFamily", "system-ui"),
];

const SCREEN_FRAME_CHECK_INTERVAL: f64 = 2_500.0;
const SCREEN_FRAME_ROUNDING: f64 = 10.0;

/// Loads the v5.2.0 font availability source.
pub fn source_fonts(_options: JsValue) -> JsValue {
    let action = callback2(|_iframe, iframe_window| collect_fonts(iframe_window));
    with_iframe(action, JsValue::undefined(), JsValue::erase(50.0_f64))
}

fn collect_fonts(iframe_window: JsValue) -> JsValue {
    let iframe_document = get(iframe_window, "document");
    let holder = get(iframe_document, "body");
    set(get(holder, "style"), "fontSize", JsValue::erase(TEXT_SIZE));

    let container = create_element(iframe_document, "div");
    set_style_property(get(container, "style"), "visibility", "hidden", "important");

    let mut base_spans = [JsValue::undefined(); 3];
    for (index, family) in BASE_FONTS.iter().enumerate() {
        base_spans[index] = create_font_span(iframe_document, container, JsValue::erase(*family));
    }

    let font_spans = array();
    for (font_index, font) in FONT_LIST.iter().enumerate() {
        let spans = array();
        for base in BASE_FONTS {
            let family = join_as_strings(&[
                JsValue::erase("'"),
                JsValue::erase(*font),
                JsValue::erase("',"),
                JsValue::erase(base),
            ]);
            push(spans, create_font_span(iframe_document, container, family));
        }
        set_index(font_spans, font_index, spans);
    }

    let _ = append_child(holder, container);
    let mut default_width = [0.0; 3];
    let mut default_height = [0.0; 3];
    for index in 0..3 {
        default_width[index] = number_value(get(base_spans[index], "offsetWidth"));
        default_height[index] = number_value(get(base_spans[index], "offsetHeight"));
    }

    let available = array();
    for (font_index, font) in FONT_LIST.iter().enumerate() {
        let spans = crate::index(font_spans, font_index);
        let mut found = false;
        for base_index in 0..3 {
            let span = crate::index(spans, base_index);
            if number_value(get(span, "offsetWidth")) != default_width[base_index]
                || number_value(get(span, "offsetHeight")) != default_height[base_index]
            {
                found = true;
                break;
            }
        }
        if found {
            push(available, JsValue::erase(*font));
        }
    }
    available
}

fn create_font_span(document: JsValue, container: JsValue, family: JsValue) -> JsValue {
    let span = create_element(document, "span");
    let style = get(span, "style");
    set(style, "position", JsValue::erase("absolute"));
    set(style, "top", JsValue::erase("0"));
    set(style, "left", JsValue::erase("0"));
    set(style, "fontFamily", family);
    set(span, "textContent", JsValue::erase(TEST_STRING));
    let _ = append_child(container, span);
    span
}

/// Loads the v5.2.0 DOM blocker source.
pub fn source_dom_blockers(options: JsValue) -> JsValue {
    let debug = !is_nullish(options) && get(options, "debug").is_truthy();
    if !dom_blockers_applicable() {
        return resolved_promise(JsValue::undefined());
    }
    let finish = callback1(move |blocked| {
        if debug {
            print_blocker_debug(blocked);
        }
        let active = array();
        for (name, selectors) in BLOCKER_FILTERS {
            let blocked_count = selectors
                .iter()
                .filter(|selector| get(blocked, selector).is_truthy())
                .count();
            if (blocked_count as f64) > (selectors.len() as f64) * 0.6 {
                push(active, JsValue::erase(*name));
            }
        }
        sort(active);
        active
    });
    call(get_blocked_selectors(), "then", &[finish])
}

/// Applies the source's exact WebKit-or-Android applicability rule.
pub fn dom_blockers_applicable() -> bool {
    dom_is_webkit() || dom_is_android()
}

fn get_blocked_selectors() -> JsValue {
    let host_document = document();
    let root = create_element(host_document, "div");
    let elements = array();
    let blocked = object();
    force_show(root);

    let mut selector_index = 0;
    for (_, selectors) in BLOCKER_FILTERS {
        for selector in *selectors {
            let element = match selector_to_element(host_document, selector) {
                Ok(element) => element,
                Err(error) => return rejected_promise(error),
            };
            if host_string(get(element, "tagName")) == "DIALOG" {
                let _ = call(element, "show", &[]);
            }
            let holder = create_element(host_document, "div");
            force_show(holder);
            let _ = append_child(holder, element);
            let _ = append_child(root, holder);
            set_index(elements, selector_index, element);
            selector_index += 1;
        }
    }

    let measure = callback0(move || {
        let _ = append_child(get(host_document, "body"), root);
        let mut selector_index = 0;
        for (_, selectors) in BLOCKER_FILTERS {
            for selector in *selectors {
                if !get(crate::index(elements, selector_index), "offsetParent").is_truthy() {
                    set(blocked, selector, JsValue::erase(true));
                }
                selector_index += 1;
            }
        }
        remove_from_parent(root);
        blocked
    });
    call(wait_for_document_body(50.0), "then", &[measure])
}

fn selector_to_element(host_document: JsValue, selector: &str) -> Result<JsValue, JsValue> {
    let parsed = match parse_blocker_selector(selector) {
        Some(parsed) => parsed,
        None => {
            return Err(crate::new_error_host(
                "Error",
                join_as_strings(&[
                    JsValue::erase("Unexpected syntax '"),
                    JsValue::erase(selector),
                    JsValue::erase("'"),
                ]),
            ));
        }
    };
    let element = create_element(host_document, parsed.tag.unwrap_or("div"));
    for (name, values) in &parsed.attributes {
        let joined = array();
        for value in values {
            push(joined, JsValue::erase(*value));
        }
        let value = call(joined, "join", &[JsValue::erase(" ")]);
        if *name == "style" {
            add_style_string(get(element, "style"), value.read());
        } else {
            let _ = call(
                element,
                "setAttribute",
                &[JsValue::erase(*name), value],
            );
        }
    }
    Ok(element)
}

struct BlockerSelector<'a> {
    tag: Option<&'a str>,
    attributes: Vec<(&'a str, Vec<&'a str>)>,
}

fn parse_blocker_selector(selector: &str) -> Option<BlockerSelector<'_>> {
    let trimmed = selector.trim_start();
    let tag_end = trimmed
        .bytes()
        .position(|byte| !byte.is_ascii_alphabetic() && byte != b'-')
        .unwrap_or(trimmed.len());
    let tag = (tag_end > 0).then_some(&trimmed[..tag_end]);
    let rest = &trimmed[tag_end..];
    let bytes = rest.as_bytes();
    let mut attributes = Vec::<(&str, Vec<&str>)>::new();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            prefix @ (b'.' | b':' | b'#')
                if bytes
                    .get(index + 1)
                    .is_some_and(|byte| is_selector_word(*byte)) =>
            {
                let start = index + 1;
                index = start;
                while index < bytes.len() && is_selector_word(bytes[index]) {
                    index += 1;
                }
                let name = match prefix {
                    b'.' => "class",
                    b'#' => "id",
                    _ => return None,
                };
                add_selector_attribute(&mut attributes, name, &rest[start..index]);
            }
            b'[' => {
                let mut end = index + 1;
                while end < bytes.len() && bytes[end] != b']' {
                    end += 1;
                }
                if end == bytes.len() {
                    index += 1;
                    continue;
                }
                let (name, value) = parse_blocker_attribute(&rest[index + 1..end])?;
                add_selector_attribute(&mut attributes, name, value);
                index = end + 1;
            }
            _ => index += 1,
        }
    }

    Some(BlockerSelector { tag, attributes })
}

fn add_selector_attribute<'a>(
    attributes: &mut Vec<(&'a str, Vec<&'a str>)>,
    name: &'a str,
    value: &'a str,
) {
    if let Some((_, values)) = attributes
        .iter_mut()
        .find(|(attribute_name, _)| *attribute_name == name)
    {
        values.push(value);
    } else {
        attributes.push((name, alloc::vec![value]));
    }
}

fn parse_blocker_attribute(attribute: &str) -> Option<(&str, &str)> {
    let bytes = attribute.as_bytes();
    let mut index = 0;
    while index < bytes.len() && is_selector_word(bytes[index]) {
        index += 1;
    }
    if index == 0 {
        return None;
    }
    let name = &attribute[..index];
    let mut value = "";

    if index < bytes.len() && !bytes[index].is_ascii_whitespace() {
        if matches!(bytes[index], b'~' | b'|' | b'^' | b'$' | b'*') {
            index += 1;
        }
        if index == bytes.len() || bytes[index] != b'=' {
            return None;
        }
        index += 1;
        if index < bytes.len() && bytes[index] == b'"' {
            index += 1;
            let start = index;
            while index < bytes.len() && bytes[index] != b'"' {
                index += 1;
            }
            if index == bytes.len() {
                return None;
            }
            value = &attribute[start..index];
            index += 1;
        } else {
            let start = index;
            while index < bytes.len() && is_selector_word(bytes[index]) {
                index += 1;
            }
            if index == start {
                return None;
            }
            value = &attribute[start..index];
        }
    }

    if index < bytes.len() {
        if !bytes[index].is_ascii_whitespace() {
            return None;
        }
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index == bytes.len() || !matches!(bytes[index], b'i' | b's') {
            return None;
        }
        index += 1;
    }
    (index == bytes.len()).then_some((name, value))
}

fn is_selector_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn force_show(element: JsValue) {
    let style = get(element, "style");
    set_style_property(style, "visibility", "hidden", "important");
    set_style_property(style, "display", "block", "important");
}

fn print_blocker_debug(blocked: JsValue) {
    let parts = array();
    push(parts, JsValue::erase("DOM blockers debug:\n```"));
    for (name, selectors) in BLOCKER_FILTERS {
        push(parts, JsValue::erase("\n"));
        push(parts, JsValue::erase(*name));
        push(parts, JsValue::erase(":"));
        for selector in *selectors {
            push(parts, JsValue::erase("\n  "));
            push(
                parts,
                JsValue::erase(if get(blocked, selector).is_truthy() {
                    "\u{1f6ab}"
                } else {
                    "\u{27a1}\u{fe0f}"
                }),
            );
            push(parts, JsValue::erase(" "));
            push(parts, JsValue::erase(*selector));
        }
    }
    push(parts, JsValue::erase("\n```"));
    let console = get(global_this(), "console");
    let _ = call(console, "log", &[call(parts, "join", &[JsValue::erase("")])]);
}

/// Loads the v5.2.0 font preference source.
pub fn source_font_preferences(_options: JsValue) -> JsValue {
    let action = callback2(|_iframe, iframe_window| {
        prepare_natural_fonts(iframe_window, 4_000);
        collect_font_preferences(iframe_window)
    });
    with_iframe(
        action,
        JsValue::erase(
            "<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
        ),
        JsValue::erase(50.0_f64),
    )
}

fn prepare_natural_fonts(iframe_window: JsValue, container_width: usize) {
    let iframe_document = get(iframe_window, "document");
    let body = get(iframe_document, "body");
    let body_style = get(body, "style");
    set(body_style, "width", JsValue::erase("4000px"));
    let _ = container_width;
    set(body_style, "webkitTextSizeAdjust", JsValue::erase("none"));
    set(body_style, "textSizeAdjust", JsValue::erase("none"));
    if dom_is_chromium() {
        let ratio = number_value(get(iframe_window, "devicePixelRatio"));
        set(
            body_style,
            "zoom",
            string_value(JsValue::erase(1.0 / ratio)),
        );
    } else if dom_is_webkit() {
        set(body_style, "zoom", JsValue::erase("reset"));
    }

    let lines = create_element(iframe_document, "div");
    let text = natural_font_text(container_width);
    set(lines, "textContent", JsValue::erase(text));
    let _ = append_child(body, lines);
}

fn collect_font_preferences(iframe_window: JsValue) -> JsValue {
    let iframe_document = get(iframe_window, "document");
    let container = get(iframe_document, "body");
    let mut elements = [JsValue::undefined(); 7];

    for (index, (_, property, value)) in FONT_PREFERENCES.iter().enumerate() {
        let element = create_element(iframe_document, "span");
        set(element, "textContent", JsValue::erase(FONT_PREFERENCE_TEXT));
        let style = get(element, "style");
        set(style, "whiteSpace", JsValue::erase("nowrap"));
        if !property.is_empty() {
            set(style, property, JsValue::erase(*value));
        }
        elements[index] = element;
        let line_break = create_element(iframe_document, "br");
        let _ = call(container, "append", &[line_break, element]);
    }

    let normalize = dom_is_chromium() && dom_is_chromium_128_or_newer();
    let ratio = number_value(get(iframe_window, "devicePixelRatio"));
    let sizes = object();
    for (index, (name, _, _)) in FONT_PREFERENCES.iter().enumerate() {
        let rectangle = call(elements[index], "getBoundingClientRect", &[]);
        let raw_width = number_value(get(rectangle, "width"));
        let width = if normalize {
            normalize_font_measurement(raw_width * ratio)
        } else {
            raw_width
        };
        set(sizes, name, JsValue::erase(width));
    }
    sizes
}

fn normalize_font_measurement(value: f64) -> f64 {
    let power = if dom_is_android() { 1.0 } else { 1_000.0 };
    let math = get(global_this(), "Math");
    number_value(call(math, "floor", &[JsValue::erase(value * power)])) / power
}

/// Module-lifetime state required by the screen-frame source.
#[derive(Clone, Default)]
pub struct ScreenFrameState(Rc<RefCell<ScreenFrameInner>>);

#[derive(Default)]
struct ScreenFrameInner {
    backup: Option<[Option<f64>; 4]>,
    timeout: Option<f64>,
}

impl ScreenFrameState {
    /// Creates isolated state. The package facade should retain one instance for the module.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the watch and cached value. This matches upstream's test-only reset.
    pub fn reset(&self) {
        let timeout = self.0.borrow_mut().timeout.take();
        if let Some(timeout) = timeout {
            crate::clear_timeout(timeout);
        }
        self.0.borrow_mut().backup = None;
    }
}

/// Loads stabilized screen-frame collection with caller-retained module state.
pub fn source_screen_frame_with_state(_options: JsValue, state: ScreenFrameState) -> JsValue {
    if (dom_is_webkit() && dom_is_webkit_616_or_newer() && dom_is_safari_webkit())
        || (dom_is_gecko() && dom_is_gecko_143_or_newer())
    {
        return callback0(|| resolved_promise(JsValue::undefined()));
    }
    let unstable = unstable_screen_frame_with_state(state);
    callback0(move || {
        let finish = callback1(move |frame| {
            for index in 0..4 {
                let value = crate::index(frame, index);
                if !value.is_null() {
                    set_index(
                        frame,
                        index,
                        JsValue::erase(round(number_value(value), SCREEN_FRAME_ROUNDING)),
                    );
                }
            }
            frame
        });
        call(
            as_promise(invoke(unstable, JsValue::undefined(), &[])),
            "then",
            &[finish],
        )
    })
}

/// Loads screen-frame collection with isolated state for integration fallback.
pub fn source_screen_frame(options: JsValue) -> JsValue {
    source_screen_frame_with_state(options, ScreenFrameState::new())
}

/// Returns the unstable screen-frame getter with caller-retained module state.
pub fn unstable_screen_frame_with_state(state: ScreenFrameState) -> JsValue {
    watch_screen_frame(&state);
    callback0(move || {
        let state = state.clone();
        let frame = current_screen_frame();
        if frame_is_null(frame) {
            if let Some(backup) = state.0.borrow().backup {
                return resolved_promise(frame_array(backup));
            }
            if fullscreen_element().is_truthy() {
                return match exit_fullscreen_dom() {
                    Ok(exited) => {
                        let after = callback0(move || {
                            let frame = current_screen_frame();
                            if !frame_is_null(frame) {
                                state.0.borrow_mut().backup = Some(frame);
                            }
                            frame_array(frame)
                        });
                        call(as_promise(exited), "then", &[after])
                    }
                    Err(error) => rejected_promise(error),
                };
            }
        }
        if !frame_is_null(frame) {
            state.0.borrow_mut().backup = Some(frame);
        }
        resolved_promise(frame_array(frame))
    })
}

/// Returns the unstable screen-frame getter with isolated state.
pub fn get_unstable_screen_frame() -> JsValue {
    unstable_screen_frame_with_state(ScreenFrameState::new())
}

fn watch_screen_frame(state: &ScreenFrameState) {
    if state.0.borrow().timeout.is_some() {
        return;
    }
    check_screen_frame(state);
}

fn check_screen_frame(state: &ScreenFrameState) {
    let frame = current_screen_frame();
    if frame_is_null(frame) {
        let callback_state = state.clone();
        let callback = callback0(move || {
            callback_state.0.borrow_mut().timeout = None;
            check_screen_frame(&callback_state);
            JsValue::undefined()
        });
        let timeout = crate::set_timeout(callback, SCREEN_FRAME_CHECK_INTERVAL);
        state.0.borrow_mut().timeout = Some(timeout);
    } else {
        let mut state = state.0.borrow_mut();
        state.backup = Some(frame);
        state.timeout = None;
    }
}

fn current_screen_frame() -> [Option<f64>; 4] {
    let host_screen = screen();
    let top = finite(parse_float(get(host_screen, "availTop")));
    let left = finite(parse_float(get(host_screen, "availLeft")));
    let width = parse_float(get(host_screen, "width"))
        - parse_float(get(host_screen, "availWidth"))
        - left.unwrap_or(0.0);
    let height = parse_float(get(host_screen, "height"))
        - parse_float(get(host_screen, "availHeight"))
        - top.unwrap_or(0.0);
    [top, finite(width), finite(height), left]
}

fn finite(value: f64) -> Option<f64> {
    (!value.is_nan()).then_some(value)
}

fn frame_is_null(frame: [Option<f64>; 4]) -> bool {
    let empty = |value: Option<f64>| value.is_none_or(|value| value == 0.0);
    empty(frame[0]) && empty(frame[1]) && empty(frame[2]) && empty(frame[3])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_the_oracle_inventory() {
        assert_eq!(FONT_LIST.len(), 52);
        assert_eq!(BLOCKER_FILTERS.len(), 45);
        assert_eq!(
            BLOCKER_FILTERS
                .iter()
                .map(|(_, selectors)| selectors.len())
                .sum::<usize>(),
            205
        );
        assert_eq!(FONT_PREFERENCES.len(), 7);
    }

    #[test]
    fn frame_null_check_uses_javascript_truthiness_for_numbers() {
        assert!(frame_is_null([None, Some(0.0), Some(-0.0), None]));
        assert!(!frame_is_null([None, Some(1.0), None, None]));
        assert!(!frame_is_null([None, Some(-1.0), None, None]));
    }

    #[test]
    fn parses_every_pinned_blocker_selector_without_btree_map() {
        for (_, selectors) in BLOCKER_FILTERS {
            for selector in *selectors {
                assert!(parse_blocker_selector(selector).is_some(), "{selector}");
            }
        }
        let parsed = parse_blocker_selector("#id.a.b[title=word i]").unwrap();
        assert_eq!(parsed.tag, None);
        assert_eq!(parsed.attributes[0], ("id", alloc::vec!["id"]));
        assert_eq!(parsed.attributes[1], ("class", alloc::vec!["a", "b"]));
        assert_eq!(parsed.attributes[2], ("title", alloc::vec!["word"]));
        assert!(parse_blocker_selector(":unsupported").is_none());
    }
}
