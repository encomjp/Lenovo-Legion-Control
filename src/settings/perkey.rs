//! Per-key Spectrum painter — DE QWERTZ and US QWERTY layouts.

use crate::widgets::tip;
use gtk::prelude::*;
use gtk::{cairo, glib, Align, GestureClick, Orientation};
use gtk4 as gtk;
use legion_core::keyboard::{color_key_for_code, KeyboardLayout};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy)]
struct KeyGeom {
    /// Stable Spectrum LED code (layout-independent).
    code: u16,
    label: &'static str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// DE ISO QWERTZ (verified 83RU).
const LAYOUT_DE: &[KeyGeom] = &[
    KeyGeom {
        code: 0x0001,
        label: "Esc",
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0002,
        label: "F1",
        x: 1.3,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0003,
        label: "F2",
        x: 2.3,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0004,
        label: "F3",
        x: 3.3,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0005,
        label: "F4",
        x: 4.3,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0006,
        label: "F5",
        x: 5.6,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0007,
        label: "F6",
        x: 6.6,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0008,
        label: "F7",
        x: 7.6,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0009,
        label: "F8",
        x: 8.6,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000a,
        label: "F9",
        x: 9.9,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000b,
        label: "F10",
        x: 10.9,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000c,
        label: "F11",
        x: 11.9,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000d,
        label: "F12",
        x: 12.9,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000f,
        label: "Prt",
        x: 14.2,
        y: 0.0,
        w: 0.9,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000e,
        label: "Ins",
        x: 15.15,
        y: 0.0,
        w: 0.9,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0010,
        label: "Del",
        x: 16.1,
        y: 0.0,
        w: 0.9,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0016,
        label: "^",
        x: 0.0,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0017,
        label: "1",
        x: 1.05,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0018,
        label: "2",
        x: 2.1,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0019,
        label: "3",
        x: 3.15,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001a,
        label: "4",
        x: 4.2,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001b,
        label: "5",
        x: 5.25,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001c,
        label: "6",
        x: 6.3,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001d,
        label: "7",
        x: 7.35,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001e,
        label: "8",
        x: 8.4,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001f,
        label: "9",
        x: 9.45,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0020,
        label: "0",
        x: 10.5,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0021,
        label: "ß",
        x: 11.55,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0022,
        label: "´",
        x: 12.6,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0038,
        label: "Bksp",
        x: 13.65,
        y: 1.0,
        w: 2.05,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0011,
        label: "Hm",
        x: 16.0,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0040,
        label: "Tab",
        x: 0.0,
        y: 2.1,
        w: 1.5,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0042,
        label: "Q",
        x: 1.55,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0043,
        label: "W",
        x: 2.6,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0044,
        label: "E",
        x: 3.65,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0045,
        label: "R",
        x: 4.7,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0046,
        label: "T",
        x: 5.75,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0047,
        label: "Z",
        x: 6.8,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0048,
        label: "U",
        x: 7.85,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0049,
        label: "I",
        x: 8.9,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004a,
        label: "O",
        x: 9.95,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004b,
        label: "P",
        x: 11.0,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004c,
        label: "Ü",
        x: 12.05,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004d,
        label: "+",
        x: 13.1,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0077,
        label: "Enter",
        x: 14.15,
        y: 2.1,
        w: 1.55,
        h: 2.1,
    },
    KeyGeom {
        code: 0x0013,
        label: "PgU",
        x: 16.0,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0055,
        label: "Caps",
        x: 0.0,
        y: 3.2,
        w: 1.75,
        h: 1.0,
    },
    KeyGeom {
        code: 0x006d,
        label: "A",
        x: 1.8,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x006e,
        label: "S",
        x: 2.85,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0058,
        label: "D",
        x: 3.9,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0059,
        label: "F",
        x: 4.95,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x005a,
        label: "G",
        x: 6.0,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0071,
        label: "H",
        x: 7.05,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0072,
        label: "J",
        x: 8.1,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x005b,
        label: "K",
        x: 9.15,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x005c,
        label: "L",
        x: 10.2,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x005d,
        label: "Ö",
        x: 11.25,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x005f,
        label: "Ä",
        x: 12.3,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004e,
        label: "#",
        x: 13.35,
        y: 3.2,
        w: 0.75,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0014,
        label: "PgD",
        x: 16.0,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x006a,
        label: "Shift",
        x: 0.0,
        y: 4.3,
        w: 1.25,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004e,
        label: "<>",
        x: 1.3,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0082,
        label: "Y",
        x: 2.35,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0083,
        label: "X",
        x: 3.4,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x006f,
        label: "C",
        x: 4.45,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0070,
        label: "V",
        x: 5.5,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0087,
        label: "B",
        x: 6.55,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0088,
        label: "N",
        x: 7.6,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0073,
        label: "M",
        x: 8.65,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0074,
        label: ",",
        x: 9.7,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0075,
        label: ".",
        x: 10.75,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0076,
        label: "-",
        x: 11.8,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x008d,
        label: "Shift",
        x: 12.85,
        y: 4.3,
        w: 2.85,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0012,
        label: "End",
        x: 16.0,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0080,
        label: "Fn",
        x: 0.0,
        y: 5.4,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x007f,
        label: "Ctrl",
        x: 1.05,
        y: 5.4,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0096,
        label: "Win",
        x: 2.1,
        y: 5.4,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0097,
        label: "Alt",
        x: 3.15,
        y: 5.4,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0098,
        label: "",
        x: 4.2,
        y: 5.4,
        w: 4.55,
        h: 1.0,
    },
    KeyGeom {
        code: 0x009a,
        label: "AltGr",
        x: 8.8,
        y: 5.4,
        w: 1.15,
        h: 1.0,
    },
    KeyGeom {
        code: 0x009b,
        label: "Menu",
        x: 10.0,
        y: 5.4,
        w: 1.1,
        h: 1.0,
    },
    KeyGeom {
        code: 0x009c,
        label: "<",
        x: 12.5,
        y: 5.55,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x009d,
        label: "^",
        x: 13.5,
        y: 5.2,
        w: 0.95,
        h: 0.55,
    },
    KeyGeom {
        code: 0x009f,
        label: "v",
        x: 13.5,
        y: 5.8,
        w: 0.95,
        h: 0.55,
    },
    KeyGeom {
        code: 0x00a1,
        label: ">",
        x: 14.5,
        y: 5.55,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0026,
        label: "Num",
        x: 17.3,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0027,
        label: "/",
        x: 18.35,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0028,
        label: "*",
        x: 19.4,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0029,
        label: "-",
        x: 20.45,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004f,
        label: "7",
        x: 17.3,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0050,
        label: "8",
        x: 18.35,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0051,
        label: "9",
        x: 19.4,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0090,
        label: "+",
        x: 20.45,
        y: 2.1,
        w: 1.0,
        h: 2.1,
    },
    KeyGeom {
        code: 0x0079,
        label: "4",
        x: 17.3,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x007b,
        label: "5",
        x: 18.35,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x007c,
        label: "6",
        x: 19.4,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x008e,
        label: "1",
        x: 17.3,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0068,
        label: "2",
        x: 18.35,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0092,
        label: "3",
        x: 19.4,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x00a7,
        label: "Enter",
        x: 20.45,
        y: 4.3,
        w: 1.0,
        h: 2.1,
    },
    KeyGeom {
        code: 0x00a3,
        label: "0",
        x: 17.3,
        y: 5.4,
        w: 2.05,
        h: 1.0,
    },
    KeyGeom {
        code: 0x00a5,
        label: ",",
        x: 19.4,
        y: 5.4,
        w: 1.0,
        h: 1.0,
    },
];

/// US ANSI QWERTY — same LED codes, US labels (no ISO <> key; wider LShift).
const LAYOUT_US: &[KeyGeom] = &[
    KeyGeom {
        code: 0x0001,
        label: "Esc",
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0002,
        label: "F1",
        x: 1.3,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0003,
        label: "F2",
        x: 2.3,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0004,
        label: "F3",
        x: 3.3,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0005,
        label: "F4",
        x: 4.3,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0006,
        label: "F5",
        x: 5.6,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0007,
        label: "F6",
        x: 6.6,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0008,
        label: "F7",
        x: 7.6,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0009,
        label: "F8",
        x: 8.6,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000a,
        label: "F9",
        x: 9.9,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000b,
        label: "F10",
        x: 10.9,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000c,
        label: "F11",
        x: 11.9,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000d,
        label: "F12",
        x: 12.9,
        y: 0.0,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000f,
        label: "Prt",
        x: 14.2,
        y: 0.0,
        w: 0.9,
        h: 0.7,
    },
    KeyGeom {
        code: 0x000e,
        label: "Ins",
        x: 15.15,
        y: 0.0,
        w: 0.9,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0010,
        label: "Del",
        x: 16.1,
        y: 0.0,
        w: 0.9,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0016,
        label: "`",
        x: 0.0,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0017,
        label: "1",
        x: 1.05,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0018,
        label: "2",
        x: 2.1,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0019,
        label: "3",
        x: 3.15,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001a,
        label: "4",
        x: 4.2,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001b,
        label: "5",
        x: 5.25,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001c,
        label: "6",
        x: 6.3,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001d,
        label: "7",
        x: 7.35,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001e,
        label: "8",
        x: 8.4,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x001f,
        label: "9",
        x: 9.45,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0020,
        label: "0",
        x: 10.5,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0021,
        label: "-",
        x: 11.55,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0022,
        label: "=",
        x: 12.6,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0038,
        label: "Bksp",
        x: 13.65,
        y: 1.0,
        w: 2.05,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0011,
        label: "Hm",
        x: 16.0,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0040,
        label: "Tab",
        x: 0.0,
        y: 2.1,
        w: 1.5,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0042,
        label: "Q",
        x: 1.55,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0043,
        label: "W",
        x: 2.6,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0044,
        label: "E",
        x: 3.65,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0045,
        label: "R",
        x: 4.7,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0046,
        label: "T",
        x: 5.75,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0047,
        label: "Y",
        x: 6.8,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0048,
        label: "U",
        x: 7.85,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0049,
        label: "I",
        x: 8.9,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004a,
        label: "O",
        x: 9.95,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004b,
        label: "P",
        x: 11.0,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004c,
        label: "[",
        x: 12.05,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004d,
        label: "]",
        x: 13.1,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004e,
        label: "\\",
        x: 14.15,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0013,
        label: "PgU",
        x: 16.0,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0055,
        label: "Caps",
        x: 0.0,
        y: 3.2,
        w: 1.75,
        h: 1.0,
    },
    KeyGeom {
        code: 0x006d,
        label: "A",
        x: 1.8,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x006e,
        label: "S",
        x: 2.85,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0058,
        label: "D",
        x: 3.9,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0059,
        label: "F",
        x: 4.95,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x005a,
        label: "G",
        x: 6.0,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0071,
        label: "H",
        x: 7.05,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0072,
        label: "J",
        x: 8.1,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x005b,
        label: "K",
        x: 9.15,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x005c,
        label: "L",
        x: 10.2,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x005d,
        label: ";",
        x: 11.25,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x005f,
        label: "'",
        x: 12.3,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0077,
        label: "Enter",
        x: 13.35,
        y: 3.2,
        w: 2.35,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0014,
        label: "PgD",
        x: 16.0,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x006a,
        label: "Shift",
        x: 0.0,
        y: 4.3,
        w: 2.25,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0082,
        label: "Z",
        x: 2.35,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0083,
        label: "X",
        x: 3.4,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x006f,
        label: "C",
        x: 4.45,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0070,
        label: "V",
        x: 5.5,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0087,
        label: "B",
        x: 6.55,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0088,
        label: "N",
        x: 7.6,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0073,
        label: "M",
        x: 8.65,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0074,
        label: ",",
        x: 9.7,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0075,
        label: ".",
        x: 10.75,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0076,
        label: "/",
        x: 11.8,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x008d,
        label: "Shift",
        x: 12.85,
        y: 4.3,
        w: 2.85,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0012,
        label: "End",
        x: 16.0,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0080,
        label: "Fn",
        x: 0.0,
        y: 5.4,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x007f,
        label: "Ctrl",
        x: 1.05,
        y: 5.4,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0096,
        label: "Win",
        x: 2.1,
        y: 5.4,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0097,
        label: "Alt",
        x: 3.15,
        y: 5.4,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0098,
        label: "",
        x: 4.2,
        y: 5.4,
        w: 5.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x009a,
        label: "Alt",
        x: 9.25,
        y: 5.4,
        w: 1.1,
        h: 1.0,
    },
    KeyGeom {
        code: 0x009b,
        label: "Menu",
        x: 10.4,
        y: 5.4,
        w: 1.1,
        h: 1.0,
    },
    KeyGeom {
        code: 0x009c,
        label: "<",
        x: 12.5,
        y: 5.55,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x009d,
        label: "^",
        x: 13.5,
        y: 5.2,
        w: 0.95,
        h: 0.55,
    },
    KeyGeom {
        code: 0x009f,
        label: "v",
        x: 13.5,
        y: 5.8,
        w: 0.95,
        h: 0.55,
    },
    KeyGeom {
        code: 0x00a1,
        label: ">",
        x: 14.5,
        y: 5.55,
        w: 0.95,
        h: 0.7,
    },
    KeyGeom {
        code: 0x0026,
        label: "Num",
        x: 17.3,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0027,
        label: "/",
        x: 18.35,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0028,
        label: "*",
        x: 19.4,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0029,
        label: "-",
        x: 20.45,
        y: 1.0,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x004f,
        label: "7",
        x: 17.3,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0050,
        label: "8",
        x: 18.35,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0051,
        label: "9",
        x: 19.4,
        y: 2.1,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0090,
        label: "+",
        x: 20.45,
        y: 2.1,
        w: 1.0,
        h: 2.1,
    },
    KeyGeom {
        code: 0x0079,
        label: "4",
        x: 17.3,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x007b,
        label: "5",
        x: 18.35,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x007c,
        label: "6",
        x: 19.4,
        y: 3.2,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x008e,
        label: "1",
        x: 17.3,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0068,
        label: "2",
        x: 18.35,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x0092,
        label: "3",
        x: 19.4,
        y: 4.3,
        w: 1.0,
        h: 1.0,
    },
    KeyGeom {
        code: 0x00a7,
        label: "Enter",
        x: 20.45,
        y: 4.3,
        w: 1.0,
        h: 2.1,
    },
    KeyGeom {
        code: 0x00a3,
        label: "0",
        x: 17.3,
        y: 5.4,
        w: 2.05,
        h: 1.0,
    },
    KeyGeom {
        code: 0x00a5,
        label: ".",
        x: 19.4,
        y: 5.4,
        w: 1.0,
        h: 1.0,
    },
];

const LAYOUT_W: f64 = 21.5;
const LAYOUT_H: f64 = 6.6;

fn layout_keys(layout: KeyboardLayout) -> &'static [KeyGeom] {
    match layout {
        KeyboardLayout::De => LAYOUT_DE,
        KeyboardLayout::Us => LAYOUT_US,
    }
}

/// Rec.601 luminance threshold used to pick per-key label contrast colour.
const LUMINANCE_THRESHOLD: f64 = 140.0;

fn contrast_label(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let lum = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
    if lum > LUMINANCE_THRESHOLD {
        (0.08, 0.08, 0.1)
    } else {
        (0.95, 0.95, 0.97)
    }
}

fn hit_test(keys: &[KeyGeom], ux: f64, uy: f64) -> Option<&KeyGeom> {
    keys.iter()
        .rev()
        .find(|k| ux >= k.x && ux <= k.x + k.w && uy >= k.y && uy <= k.y + k.h)
}

/// Normalize legacy name keys → hex code keys.
fn normalize_color_map(raw: HashMap<String, [u8; 3]>) -> HashMap<String, [u8; 3]> {
    let mut out = HashMap::new();
    for (k, rgb) in raw {
        if let Some(code) = legion_core::keyboard::keycode_by_name(&k) {
            out.insert(color_key_for_code(code), rgb);
        } else if k.starts_with("0x") || k.starts_with("0X") {
            out.insert(k.to_lowercase(), rgb);
        }
    }
    out
}

pub fn build_perkey_editor(paint: Rc<Cell<(u8, u8, u8)>>) -> gtk::Box {
    let root = gtk::Box::new(Orientation::Vertical, 14);
    root.add_css_class("perkey-root");

    let cfg = legion_core::config::get();
    let layout = Rc::new(Cell::new(
        KeyboardLayout::from_name(&cfg.keyboard_layout).unwrap_or(KeyboardLayout::De),
    ));

    let colors: Rc<RefCell<HashMap<String, [u8; 3]>>> =
        Rc::new(RefCell::new(normalize_color_map(cfg.per_key)));

    // Layout switcher
    let switch_row = gtk::Box::new(Orientation::Horizontal, 12);
    switch_row.set_valign(Align::Center);
    let switch_l = gtk::Label::new(Some("Layout"));
    switch_l.add_css_class("row-title");
    tip(
        &switch_l,
        "Key legends only (DE QWERTZ or US QWERTY) — does not change LED wiring",
    );
    let layout_dd = gtk::DropDown::from_strings(&["DE QWERTZ", "US QWERTY"]);
    tip(
        &layout_dd,
        "Keyboard legend only — LED colours are shared between DE and US layouts",
    );
    layout_dd.set_selected(match layout.get() {
        KeyboardLayout::De => 0,
        KeyboardLayout::Us => 1,
    });
    switch_row.append(&switch_l);
    switch_row.append(&layout_dd);
    let hint = gtk::Label::new(Some("Same LEDs · different labels"));
    hint.add_css_class("hint");
    hint.set_hexpand(true);
    hint.set_halign(Align::Start);
    hint.set_margin_top(0);
    tip(&hint, "Switching layout does not wipe painted colours");
    switch_row.append(&hint);
    root.append(&switch_row);

    let area = gtk::DrawingArea::new();
    area.set_content_width(640);
    area.set_content_height(240);
    area.set_size_request(640, 240);
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.add_css_class("perkey-canvas");
    tip(
        &area,
        "Click or drag keys to paint with the colour above — switches keyboard to per-key mode",
    );

    let colors_d = colors.clone();
    let layout_d = layout.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let keys = layout_keys(layout_d.get());
        let pad = 14.0;
        let scale = ((w as f64 - pad * 2.0) / LAYOUT_W).min((h as f64 - pad * 2.0) / LAYOUT_H);
        let ox = (w as f64 - LAYOUT_W * scale) * 0.5;
        let oy = (h as f64 - LAYOUT_H * scale) * 0.5;

        cr.set_source_rgba(0.06, 0.06, 0.08, 1.0);
        let _ = cr.paint();

        let grad = cairo::LinearGradient::new(0.0, 0.0, 0.0, h as f64);
        grad.add_color_stop_rgba(0.0, 0.78, 0.06, 0.18, 0.08);
        grad.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.0);
        let _ = cr.set_source(&grad);
        let _ = cr.paint();

        let map = colors_d.borrow();
        let gap = 0.08 * scale;
        for key in keys {
            let x = ox + key.x * scale + gap * 0.5;
            let y = oy + key.y * scale + gap * 0.5;
            let kw = key.w * scale - gap;
            let kh = key.h * scale - gap;
            let radius = (7.0_f64).min(kw * 0.18).min(kh * 0.18);
            let ck = color_key_for_code(key.code);
            let (r, g, b) = map
                .get(&ck)
                .map(|c| (c[0], c[1], c[2]))
                .unwrap_or((28, 28, 34));

            rounded_rect(cr, x, y, kw, kh, radius);
            cr.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.08);
            cr.set_line_width(1.0);
            let _ = cr.stroke();

            if !key.label.is_empty() {
                let (tr, tg, tb) = contrast_label(r, g, b);
                cr.set_source_rgb(tr, tg, tb);
                cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
                let fs = (kh * 0.36).clamp(9.0, 16.0);
                cr.set_font_size(fs);
                if let Ok(ext) = cr.text_extents(key.label) {
                    cr.move_to(
                        x + (kw - ext.width()) * 0.5 - ext.x_bearing(),
                        y + (kh + ext.height()) * 0.5,
                    );
                    let _ = cr.show_text(key.label);
                }
            }
        }
    });

    let layout_sw = layout.clone();
    let area_sw = area.clone();
    layout_dd.connect_selected_notify(move |d| {
        let lay = if d.selected() == 1 {
            KeyboardLayout::Us
        } else {
            KeyboardLayout::De
        };
        layout_sw.set(lay);
        legion_core::config::set_keyboard_layout(lay.name());
        area_sw.queue_draw();
    });

    let paint_at = {
        let colors = colors.clone();
        let area = area.clone();
        let paint = paint.clone();
        let layout = layout.clone();
        let ticket = Rc::new(Cell::new(0u32));
        Rc::new(move |ux: f64, uy: f64| {
            let keys = layout_keys(layout.get());
            let Some(key) = hit_test(keys, ux, uy) else {
                return;
            };
            let (r, g, b) = paint.get();
            let ck = color_key_for_code(key.code);
            colors.borrow_mut().insert(ck.clone(), [r, g, b]);
            area.queue_draw();
            legion_core::config::set_per_key_color(&ck, r, g, b);
            let t = ticket.get().wrapping_add(1);
            ticket.set(t);
            let ticket = ticket.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(90), move || {
                if ticket.get() == t {
                    legion_core::keyboard::restore_lighting_async();
                }
            });
        })
    };

    let click = GestureClick::new();
    click.set_button(1);
    let paint_at_c = paint_at.clone();
    let area_c = area.clone();
    click.connect_pressed(move |_, _, x, y| {
        let w = area_c.width() as f64;
        let h = area_c.height() as f64;
        let pad = 14.0;
        let scale = ((w - pad * 2.0) / LAYOUT_W).min((h - pad * 2.0) / LAYOUT_H);
        let ox = (w - LAYOUT_W * scale) * 0.5;
        let oy = (h - LAYOUT_H * scale) * 0.5;
        paint_at_c((x - ox) / scale, (y - oy) / scale);
    });
    area.add_controller(click);

    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    let paint_at_d = paint_at.clone();
    let area_d = area.clone();
    let origin = Rc::new(Cell::new((0.0_f64, 0.0_f64)));
    let origin_s = origin.clone();
    drag.connect_drag_begin(move |_, x, y| {
        origin_s.set((x, y));
    });
    let origin_u = origin.clone();
    drag.connect_drag_update(move |_, dx, dy| {
        let (ox0, oy0) = origin_u.get();
        let x = ox0 + dx;
        let y = oy0 + dy;
        let w = area_d.width() as f64;
        let h = area_d.height() as f64;
        let pad = 14.0;
        let scale = ((w - pad * 2.0) / LAYOUT_W).min((h - pad * 2.0) / LAYOUT_H);
        let ox = (w - LAYOUT_W * scale) * 0.5;
        let oy = (h - LAYOUT_H * scale) * 0.5;
        paint_at_d((x - ox) / scale, (y - oy) / scale);
    });
    area.add_controller(drag);

    root.append(&area);

    let tools = gtk::Box::new(Orientation::Horizontal, 12);
    tools.add_css_class("perkey-tools");

    let clear = gtk::Button::with_label("Clear map");
    clear.add_css_class("flat");
    tip(
        &clear,
        "Remove all per-key paints and return to the whole-keyboard effect",
    );
    let colors_c = colors.clone();
    let area_cl = area.clone();
    clear.connect_clicked(move |_| {
        colors_c.borrow_mut().clear();
        area_cl.queue_draw();
        legion_core::keyboard::clear_per_key_async();
    });
    tools.append(&clear);

    let fill = gtk::Button::with_label("Fill all");
    fill.add_css_class("flat");
    tip(&fill, "Paint every key with the current brush colour");
    let colors_f = colors.clone();
    let area_f = area.clone();
    let paint_f = paint.clone();
    let layout_f = layout.clone();
    fill.connect_clicked(move |_| {
        let (r, g, b) = paint_f.get();
        {
            let mut map = colors_f.borrow_mut();
            for key in layout_keys(layout_f.get()) {
                let ck = color_key_for_code(key.code);
                map.insert(ck.clone(), [r, g, b]);
                legion_core::config::set_per_key_color(&ck, r, g, b);
            }
        }
        area_f.queue_draw();
        legion_core::keyboard::restore_lighting_async();
    });
    tools.append(&fill);

    let paint_hint = gtk::Label::new(Some("Click or drag to paint"));
    paint_hint.add_css_class("hint");
    paint_hint.set_hexpand(true);
    paint_hint.set_halign(Align::End);
    tip(&paint_hint, "Hold and drag across keys for faster painting");
    tools.append(&paint_hint);

    root.append(&tools);
    root
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w * 0.5).min(h * 0.5);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    cr.close_path();
}
