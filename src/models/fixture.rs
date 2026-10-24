//! A synthetic descriptor used by tests.
//!
//! It is deliberately different from the shipped device: per-axis DPI stages,
//! writable keymaps with combos, lighting zones, burst fire, and global device
//! settings. Those sections cannot be captured for the attached mouse yet, but
//! the code paths they drive must still be exercised.

use crate::models::Descriptor;

pub const FIXTURE_TOML: &str = r#"
schema = 1
id = "fixture-full"
model = "Fixture Mouse (test only)"
provenance = "readback"
verified = true
notes = "Synthetic descriptor used by tests; not a real product."

[collections]
usage_page = 0xff00
usage = 0x01

[[frames]]
id = 2
len = 16

[[frames]]
id = 3
len = 64

[[frames]]
id = 4
len = 1024

[profiles]
count = 5

[commands]
begin = 0xf5
write = 0xf3
read = 0xf2
commit = 0xf1
commit_report = 2
commit_frames = [0x0201]

[windows.page]
report = 3
addr = 0x0100
len = 32
stride = 0x40
count = 5

[windows.keymap]
report = 3
addr = 0x0c00
len = 32
stride = 0x0100
count = 5

[windows.combos]
report = 3
addr = 0x0c40
len = 50
stride = 0x0100
count = 5

[windows.led]
report = 2
addr = 0x0800
len = 7
stride = 8
count = 5

[windows.zones]
report = 2
addr = 0x0900
len = 3
stride = 8
count = 2
per_profile = false

[windows.polling_lo]
report = 2
addr = 0x0030
len = 4
stride = 2
count = 3

[windows.polling_hi]
report = 2
addr = 0x0036
len = 2
stride = 2
profile_base = 3
count = 2

[windows.device]
report = 2
addr = 0x0050
len = 8

[windows.macros]
report = 4
addr = 0x2000
len = 200
stride = 200
count = 15
per_profile = false

[dpi]
window = "page"
active_window_offset = 0
stage_window_offset = 4
stage_count = 5
write_report = 3
stage_stride = 6
stage_len = 5
active_len = 2
per_axis = true

[polling]
write_addrs = [0x0030, 0x0032, 0x0034, 0x0036, 0x0038]
write_len = 1

[[polling.sources]]
window = "polling_lo"

[[polling.sources]]
window = "polling_hi"

[[polling.codes]]
hz = 125
code = 0x01

[[polling.codes]]
hz = 500
code = 0x04

[[polling.codes]]
hz = 1000
code = 0x08

[lighting]
window = "led"
write_addrs = [0x0800, 0x0808, 0x0810, 0x0818, 0x0820]
color_offset = 0
mode_offset = 4
speed_offset = 3
brightness_offset = 6
brightness_levels = 4
speed_levels = 4

[[lighting.modes]]
id = "static"
label = "Static"
code = 0x08
uses_color = true

[[lighting.modes]]
id = "breathing"
label = "Breathing"
code = 0x02
uses_color = true
uses_speed = true

[[lighting.modes]]
id = "rainbow"
label = "Rainbow"
code = 0x03

[[lighting.modes]]
id = "off"
label = "Off"
code = 0x00

[[lighting.zones]]
id = "side"
label = "Side"
write_addr = 0x0900

[lighting.zones.read]
window = "zones"

[[lighting.zones]]
id = "front"
label = "Front"
write_addr = 0x0908

[lighting.zones.read]
window = "zones"

[buttons]
window = "keymap"
window_offset = 0
count = 8
names = ["Left", "Right", "Middle", "Back", "Forward", "DPI", "Sniper", "Fire"]
write_addrs = [0x0c00, 0x0d00, 0x0e00, 0x0f00, 0x1000]
write_report = 2
entry_stride = 4
entry_len = 4
primary = 0
combo_slots = 3
combo_addr = 0x0c40
combo_stride = 6
combo_profile_stride = 0x0100
combo_window = "combos"

[macros]
slots = 15
window = "macros"
base = 0x2000
slot_stride = 200
report = 4
record_len = 200
action_len = 15
max_actions = 13

[macros.burst]
addr = 0x0060
stride = 2
len = 2
max_bullets = 5
min_delay_ms = 10
max_delay_ms = 200

[device]
window = "device"

[device.angle_snap]
window_offset = 0
addr = 0x0050
len = 1
offset = 0
on = 1
off = 0

[device.lift_off]
window_offset = 1
addr = 0x0051
len = 1
offset = 0

[[device.lift_off.values]]
id = "1mm"
label = "1 mm"
code = 1

[[device.lift_off.values]]
id = "2mm"
label = "2 mm"
code = 2

[[device.lift_off.values]]
id = "3mm"
label = "3 mm"
code = 3

[device.scroll]
window_offset = 2
addr = 0x0052
len = 1
offset = 0
min = 1
max = 10

[device.scroll_page]
window_offset = 3
addr = 0x0053
len = 1
offset = 0
on = 1
off = 0

[device.accel]
window_offset = 4
addr = 0x0054
len = 1
offset = 0
on = 1
off = 0

[device.double_click]
window_offset = 5
addr = 0x0055
len = 1
offset = 0
min = 1
max = 10

[[tables.dpi_codes]]
dpi = 800
code = 0x0012

[[tables.dpi_codes]]
dpi = 1600
code = 0x0021

[[tables.dpi_codes]]
dpi = 3200
code = 0x0045

[[tables.dpi_codes]]
dpi = 6400
code = 0x0067

[[tables.actions]]
id = "default"
label = "Default"
code = 0x00
kind = "default"

[[tables.actions]]
id = "disabled"
label = "Disabled"
code = 0x01
kind = "disabled"

[[tables.actions]]
id = "left"
label = "Left click"
code = 0x02
kind = "mouse"

[[tables.actions]]
id = "right"
label = "Right click"
code = 0x03
kind = "mouse"

[[tables.actions]]
id = "dpi-cycle"
label = "DPI cycle"
code = 0x10
kind = "dpi"

[[tables.actions]]
id = "sniper"
label = "Sniper"
code = 0x11
kind = "sniper"

[[tables.actions]]
id = "profile-next"
label = "Next profile"
code = 0x12
kind = "profile"

[[tables.actions]]
id = "play-pause"
label = "Play / pause"
code = 0x20
kind = "multimedia"

[[tables.actions]]
id = "macro"
label = "Macro"
code = 0x30
kind = "macro"

[[tables.keys]]
id = "a"
label = "A"
usage = 0x04

[[tables.keys]]
id = "b"
label = "B"
usage = 0x05

[[tables.keys]]
id = "c"
label = "C"
usage = 0x06

[[tables.keys]]
id = "f5"
label = "F5"
usage = 0x3c

[[tables.keys]]
id = "ctrl"
label = "Ctrl"
usage = 0xe0
modifiers = 0x01
"#;

pub fn descriptor() -> Descriptor {
    Descriptor::from_toml(FIXTURE_TOML).expect("fixture descriptor must be valid")
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixture_descriptor_is_valid() {
        let desc = super::descriptor();
        assert!(desc.supports_buttons_write());
        assert!(desc.supports_combos());
        assert!(desc.supports_burst());
        assert!(desc.supports_device_settings());
    }
}
