//! Semantic validation for model descriptors.
//!
//! `serde` catches shape errors; this pass catches the semantic ones that would
//! otherwise turn into a wrong write: missing strides, addresses outside the
//! report capacity, tables that cannot express a declared value, and two
//! sections claiming the same bytes.

use std::collections::BTreeMap;

use super::descriptor::{Descriptor, Footprint};

pub fn validate(desc: &Descriptor) -> Vec<String> {
    let mut errors = Vec::new();
    let sizes = desc.frame_sizes();

    if desc.id.trim().is_empty() {
        errors.push("id must not be empty".into());
    }
    if desc.model.trim().is_empty() {
        errors.push("model must not be empty".into());
    }
    if desc.profiles.count == 0 {
        errors.push("profiles.count must be at least 1".into());
    }
    let mut frame_ids = BTreeMap::new();
    for frame in &desc.frames {
        if frame.len < 9 {
            errors.push(format!(
                "report {} must be at least 9 bytes, got {}",
                frame.id, frame.len
            ));
        }
        if frame_ids.insert(frame.id, frame.len).is_some() {
            errors.push(format!("report {} is declared twice", frame.id));
        }
    }
    if desc.frames.is_empty() {
        errors.push("frames must declare at least one report".into());
    }
    if sizes.for_report(desc.commands.commit_report).is_none() {
        errors.push(format!(
            "commit report {} has no declared frame size",
            desc.commands.commit_report
        ));
    }
    if desc.commands.commit_frames.is_empty() {
        errors.push("commands.commit_frames must not be empty".into());
    }
    for frame in &desc.commands.commit_frames {
        if sizes.for_report(desc.commands.commit_report).is_none() {
            errors.push(format!(
                "commit report {} has no declared frame size",
                desc.commands.commit_report
            ));
            break;
        }
        let _ = frame;
    }
    if desc.commands.activate_addr.is_some() && desc.commands.activate_frames.is_empty() {
        errors.push("commands.activate_addr requires commands.activate_frames".into());
    }

    // Windows.
    let mut instances = Vec::new();
    for (id, window) in &desc.windows {
        if window.len == 0 {
            errors.push(format!("window `{id}` has zero length"));
            continue;
        }
        match sizes.payload_capacity(window.report) {
            None => errors.push(format!(
                "window `{id}` uses report {} which has no declared frame size",
                window.report
            )),
            Some(capacity) if window.len as usize > capacity => errors.push(format!(
                "window `{id}` length {} exceeds report {} payload capacity {capacity}",
                window.len, window.report
            )),
            _ => {}
        }
        if window.per_profile
            && window.profile_base as u16 + window.count as u16 > desc.profiles.count as u16
        {
            errors.push(format!(
                "window `{id}` covers profiles {}..{} but the model declares {}",
                window.profile_base,
                window.profile_base as u16 + window.count as u16,
                desc.profiles.count
            ));
        }
        match window.expand(id) {
            Ok(list) => instances.extend(list),
            Err(err) => errors.push(err),
        }
    }

    let require_window = |errors: &mut Vec<String>, id: &str, section: &str| {
        if !desc.windows.contains_key(id) {
            errors.push(format!("{section} references unknown window `{id}`"));
        }
    };

    if let Some(dpi) = &desc.dpi {
        require_window(&mut errors, &dpi.window, "dpi");
        if dpi.stage_count == 0 {
            errors.push("dpi.stage_count must be at least 1".into());
        }
        if dpi.per_axis && dpi.stage_len < 5 {
            errors.push(format!(
                "dpi.per_axis requires stage_len >= 5, got {}",
                dpi.stage_len
            ));
        }
        if !dpi.per_axis && dpi.stage_len < 3 {
            errors.push(format!(
                "single-axis dpi requires stage_len >= 3, got {}",
                dpi.stage_len
            ));
        }
        if desc.tables.dpi_codes.is_empty() {
            errors.push("dpi is declared but tables.dpi_codes is empty".into());
        }
        // Window must contain the active value and every stage.
        if let Ok(list) = desc.window_instances_for(&dpi.window) {
            for instance in list {
                let active_end = dpi.active_window_offset as u32 + 1;
                if active_end > instance.len as u32 {
                    errors.push(format!(
                        "window `{}` is too short for the active DPI value at offset {}",
                        dpi.window, dpi.active_window_offset
                    ));
                    break;
                }
                let last_stage_offset = dpi.stage_window_offset as u32
                    + (dpi.stage_count as u32 - 1) * dpi.stage_stride as u32;
                if last_stage_offset + 3 > instance.len as u32 {
                    errors.push(format!(
                        "window `{}` length {} cannot hold stage {} at offset {}",
                        dpi.window, instance.len, dpi.stage_count, last_stage_offset
                    ));
                    break;
                }
            }
        }
        if let Ok(pages) = desc.window_instances_for(&dpi.window) {
            for page in &pages {
                let last = dpi
                    .stage_addr_at(page.addr, dpi.stage_count - 1)
                    .checked_add(dpi.stage_len as u16);
                if last.is_none() {
                    errors.push(format!(
                        "dpi stage addresses overflow for profile {}",
                        page.profile + 1
                    ));
                }
            }
        }
        match sizes.payload_capacity(dpi.write_report) {
            None => errors.push(format!(
                "dpi.write_report {} has no declared frame size",
                dpi.write_report
            )),
            Some(capacity) => {
                if dpi.stage_len as usize > capacity {
                    errors.push(format!(
                        "dpi.stage_len {} exceeds report {} capacity {capacity}",
                        dpi.stage_len, dpi.write_report
                    ));
                }
                if dpi.active_len as usize > capacity {
                    errors.push(format!(
                        "dpi.active_len {} exceeds report {} capacity {capacity}",
                        dpi.active_len, dpi.write_report
                    ));
                }
            }
        }
    } else if !desc.tables.dpi_codes.is_empty() {
        errors.push("tables.dpi_codes is declared but dpi is absent".into());
    }

    if let Some(polling) = &desc.polling {
        if polling.write_addrs.len() != desc.profiles.count as usize {
            errors.push(format!(
                "polling.write_addrs has {} entries but the model declares {} profiles",
                polling.write_addrs.len(),
                desc.profiles.count
            ));
        }
        if polling.codes.is_empty() {
            errors.push("polling.codes must not be empty".into());
        }
        for code in &polling.codes {
            if code.hz == 0 {
                errors.push("polling.codes contains a zero rate".into());
            }
        }
        let mut seen = BTreeMap::new();
        for code in &polling.codes {
            if let Some(prev) = seen.insert(code.code, code.hz) {
                errors.push(format!(
                    "polling.codes reuses code {:#04x} for {prev} Hz and {} Hz",
                    code.code, code.hz
                ));
            }
        }
        let mut profiles: Vec<u8> = Vec::new();
        for source in &polling.sources {
            require_window(&mut errors, &source.window, "polling");
            if let Ok(list) = desc.window_instances_for(&source.window) {
                for instance in list {
                    if source.offset as u32 + polling.write_len as u32 > instance.len as u32 {
                        errors.push(format!(
                            "polling window `{}` cannot hold offset {}",
                            source.window, source.offset
                        ));
                        break;
                    }
                    if instance.profile < desc.profiles.count {
                        profiles.push(instance.profile);
                    }
                }
            }
        }
        for profile in 0..desc.profiles.count {
            if !profiles.contains(&profile) {
                errors.push(format!("polling sources do not cover profile {profile}"));
            }
        }
    }

    if let Some(lighting) = &desc.lighting {
        require_window(&mut errors, &lighting.window, "lighting");
        if lighting.write_addrs.len() != desc.profiles.count as usize {
            errors.push(format!(
                "lighting.write_addrs has {} entries but the model declares {} profiles",
                lighting.write_addrs.len(),
                desc.profiles.count
            ));
        }
        if lighting.modes.is_empty() {
            errors.push("lighting.modes must not be empty".into());
        }
        let mut mode_codes = BTreeMap::new();
        for mode in &lighting.modes {
            if let Some(prev) = mode_codes.insert(mode.code, mode.id.clone()) {
                errors.push(format!(
                    "lighting.modes reuses code {:#04x} for `{prev}` and `{}`",
                    mode.code, mode.id
                ));
            }
        }
        for (id, offset, width) in [
            ("mode", lighting.mode_offset, 1u16),
            ("speed", lighting.speed_offset, 1),
            ("brightness", lighting.brightness_offset, 1),
            ("colour", lighting.color_offset, 3),
        ] {
            let mut fits = true;
            if let Ok(list) = desc.window_instances_for(&lighting.window) {
                for instance in &list {
                    if offset as u32 + width as u32 > instance.len as u32 {
                        errors.push(format!(
                            "lighting window `{}` too short for the {id} field at offset {offset}",
                            lighting.window
                        ));
                        fits = false;
                        break;
                    }
                }
            }
            if !fits {
                break;
            }
        }
        if lighting.brightness_levels == 0 {
            errors.push("lighting.brightness_levels must be at least 1".into());
        }
        if lighting.speed_levels == 0 {
            errors.push("lighting.speed_levels must be at least 1".into());
        }
        let mut zone_ids = BTreeMap::new();
        for zone in &lighting.zones {
            if zone_ids.insert(zone.id.clone(), ()).is_some() {
                errors.push(format!("lighting.zones repeats id `{}`", zone.id));
            }
            if let Some(read) = &zone.read {
                require_window(&mut errors, &read.window, "lighting.zone");
                if let Ok(list) = desc.window_instances_for(&read.window) {
                    for instance in list {
                        if read.offset as u32 + 3 > instance.len as u32 {
                            errors.push(format!(
                                "zone window `{}` too short for zone `{}` colour at offset {}",
                                read.window, zone.id, read.offset
                            ));
                            break;
                        }
                    }
                }
            }
        }
    }

    if let Some(buttons) = &desc.buttons {
        require_window(&mut errors, &buttons.window, "buttons");
        if buttons.names.len() < buttons.count as usize {
            errors.push(format!(
                "buttons.names has {} entries but count is {}",
                buttons.names.len(),
                buttons.count
            ));
        }
        if buttons.count == 0 {
            errors.push("buttons.count must be at least 1".into());
        }
        if buttons.primary >= buttons.count {
            errors.push(format!(
                "buttons.primary {} is outside 0..{}",
                buttons.primary, buttons.count
            ));
        }
        if !buttons.write_addrs.is_empty()
            && buttons.write_addrs.len() != desc.profiles.count as usize
        {
            errors.push(format!(
                "buttons.write_addrs has {} entries but the model declares {} profiles",
                buttons.write_addrs.len(),
                desc.profiles.count
            ));
        }
        if buttons.entry_len < 1 {
            errors.push("buttons.entry_len must be at least 1".into());
        }
        let needed = buttons.window_offset as u32
            + buttons.entry_stride as u32 * (buttons.count as u32 - 1)
            + buttons.entry_len as u32;
        if let Ok(list) = desc.window_instances_for(&buttons.window) {
            for instance in list {
                if needed > instance.len as u32 {
                    errors.push(format!(
                        "buttons window `{}` length {} cannot hold {} buttons",
                        buttons.window, instance.len, buttons.count
                    ));
                    break;
                }
            }
        }
        if buttons.combo_slots > 0 && buttons.combo_addr.is_none() {
            errors.push("buttons.combo_slots > 0 requires buttons.combo_addr".into());
        }
        if let Some(combo_window) = &buttons.combo_window {
            require_window(&mut errors, combo_window, "buttons.combo");
            let needed = buttons.combo_stride as u32 * (buttons.count as u32 - 1)
                + 2 * buttons.combo_slots as u32;
            if let Ok(list) = desc.window_instances_for(combo_window) {
                for instance in list {
                    if needed > instance.len as u32 {
                        errors.push(format!(
                            "combo window `{combo_window}` length {} cannot hold the combo area ({needed} bytes)",
                            instance.len
                        ));
                        break;
                    }
                }
            }
        }
        let action_kinds = desc
            .tables
            .actions
            .iter()
            .map(|a| a.kind)
            .collect::<Vec<_>>();
        if !action_kinds.is_empty() {
            use super::descriptor::ActionKind::*;
            if !action_kinds.contains(&Default) {
                errors.push("tables.actions must include a `default` action".into());
            }
            if !action_kinds.contains(&Disabled) {
                errors.push("tables.actions must include a `disabled` action".into());
            }
        }
    }

    if let Some(macros) = &desc.macros {
        require_window(&mut errors, &macros.window, "macros");
        if macros.slots == 0 {
            errors.push("macros.slots must be at least 1".into());
        }
        match sizes.payload_capacity(macros.report) {
            None => errors.push(format!(
                "macros.report {} has no declared frame size",
                macros.report
            )),
            Some(capacity) => {
                if macros.record_len as usize > capacity {
                    errors.push(format!(
                        "macros.record_len {} exceeds report {} capacity {capacity}",
                        macros.record_len, macros.report
                    ));
                }
            }
        }
        if macros.max_actions == 0 {
            errors.push("macros.max_actions must be at least 1".into());
        }
        if macros.action_len == 0 {
            errors.push("macros.action_len must be at least 1".into());
        }
        let needed = macros.max_actions as u32 * macros.action_len as u32;
        if needed > macros.record_len as u32 {
            errors.push(format!(
                "macros.max_actions {} x action_len {} = {needed} exceeds record_len {}",
                macros.max_actions, macros.action_len, macros.record_len
            ));
        }
        if let Some(burst) = &macros.burst {
            if burst.max_bullets == 0 {
                errors.push("macros.burst.max_bullets must be at least 1".into());
            }
            if burst.min_delay_ms > burst.max_delay_ms {
                errors.push("macros.burst.min_delay_ms exceeds max_delay_ms".into());
            }
            if burst.len < 2 {
                errors.push("macros.burst.len must be at least 2".into());
            }
        }
    }

    if let Some(device) = &desc.device {
        require_window(&mut errors, &device.window, "device");
        fn check(
            desc: &Descriptor,
            errors: &mut Vec<String>,
            window: &str,
            label: &str,
            window_offset: u16,
            len: u8,
        ) {
            if let Ok(list) = desc.window_instances_for(window) {
                for instance in list {
                    if window_offset as u32 + len as u32 > instance.len as u32 {
                        errors.push(format!(
                            "device window `{window}` too short for {label} at offset {window_offset}"
                        ));
                        return;
                    }
                }
            }
        }
        if let Some(f) = &device.angle_snap {
            check(
                desc,
                &mut errors,
                &device.window,
                "angle_snap",
                f.window_offset,
                f.len,
            );
        }
        if let Some(f) = &device.scroll_page {
            check(
                desc,
                &mut errors,
                &device.window,
                "scroll_page",
                f.window_offset,
                f.len,
            );
        }
        if let Some(f) = &device.accel {
            check(
                desc,
                &mut errors,
                &device.window,
                "accel",
                f.window_offset,
                f.len,
            );
        }
        if let Some(f) = &device.lift_off {
            check(
                desc,
                &mut errors,
                &device.window,
                "lift_off",
                f.window_offset,
                f.len,
            );
            if f.values.is_empty() {
                errors.push("device.lift_off.values must not be empty".into());
            }
        }
        if let Some(f) = &device.scroll {
            check(
                desc,
                &mut errors,
                &device.window,
                "scroll",
                f.window_offset,
                f.len,
            );
        }
        if let Some(f) = &device.double_click {
            check(
                desc,
                &mut errors,
                &device.window,
                "double_click",
                f.window_offset,
                f.len,
            );
            if f.min > f.max {
                errors.push("device.double_click.min exceeds max".into());
            }
        }
    }

    // Tables.
    let mut dpi_values = BTreeMap::new();
    let mut dpi_codes = BTreeMap::new();
    for entry in &desc.tables.dpi_codes {
        if dpi_values.insert(entry.dpi, entry.code).is_some() {
            errors.push(format!("tables.dpi_codes repeats DPI value {}", entry.dpi));
        }
        if dpi_codes.insert(entry.code, entry.dpi).is_some() {
            errors.push(format!(
                "tables.dpi_codes repeats register code {:#06x}",
                entry.code
            ));
        }
    }
    let mut key_ids = BTreeMap::new();
    for key in &desc.tables.keys {
        if key_ids.insert(key.id.clone(), ()).is_some() {
            errors.push(format!("tables.keys repeats id `{}`", key.id));
        }
    }
    let mut action_ids = BTreeMap::new();
    let mut action_codes = BTreeMap::new();
    for action in &desc.tables.actions {
        if action_ids.insert(action.id.clone(), ()).is_some() {
            errors.push(format!("tables.actions repeats id `{}`", action.id));
        }
        if action_codes
            .insert(action.code, action.id.clone())
            .is_some()
        {
            errors.push(format!("tables.actions repeats code {:#04x}", action.code));
        }
    }

    // Overlapping write regions (same report only).
    let footprint = desc.write_footprint();
    for (index, a) in footprint.iter().enumerate() {
        for b in footprint.iter().skip(index + 1) {
            if a.report != b.report {
                continue;
            }
            if overlaps(a, b) {
                errors.push(format!(
                    "write regions overlap on report {}: {} ({:#06x}+{}) and {} ({:#06x}+{})",
                    a.report, a.section, a.addr, a.len, b.section, b.addr, b.len
                ));
            }
        }
        if let Some(capacity) = sizes.payload_capacity(a.report) {
            if a.len as usize > capacity {
                errors.push(format!(
                    "{} writes {} bytes into report {} capacity {capacity}",
                    a.section, a.len, a.report
                ));
            }
        }
    }

    errors
}

fn overlaps(a: &Footprint, b: &Footprint) -> bool {
    // Two regions in different profiles cannot conflict: they are written
    // under different profile selections.
    if let (Some(a_profile), Some(b_profile)) = (a.profile, b.profile) {
        if a_profile != b_profile {
            return false;
        }
    }
    let a_start = a.addr as u32;
    let a_end = a_start + a.len as u32;
    let b_start = b.addr as u32;
    let b_end = b_start + b.len as u32;
    a_start < b_end && b_start < a_end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Descriptor {
        Descriptor::from_toml(include_str!("../../models/holtek-04d9-fc5c.toml"))
            .expect("bundled descriptor is valid")
    }

    #[test]
    fn bundled_descriptor_is_semantically_valid() {
        let desc = base();
        assert!(validate(&desc).is_empty(), "{:?}", validate(&desc));
    }

    #[test]
    fn missing_dpi_stride_is_reported() {
        let mut toml = include_str!("../../models/holtek-04d9-fc5c.toml").to_string();
        // Remove the stage_stride key so the section cannot address stages.
        toml = toml.replace("stage_stride = 6\n", "");
        let err = Descriptor::from_toml(&toml).expect_err("must fail");
        let text = err.to_string();
        assert!(text.contains("stage_stride"), "{text}");
    }

    #[test]
    fn overlapping_write_regions_are_reported() {
        let mut desc = base();
        // Force the LED brightness register onto the polling register.
        let polling_addr = desc.polling.as_ref().unwrap().write_addrs[0];
        let lighting = desc.lighting.as_mut().unwrap();
        lighting.write_addrs[0] = polling_addr.saturating_sub(lighting.brightness_offset);
        let errors = validate(&desc);
        assert!(errors.iter().any(|e| e.contains("overlap")), "{errors:?}");
    }

    #[test]
    fn empty_dpi_table_is_reported() {
        let mut desc = base();
        desc.tables.dpi_codes.clear();
        let errors = validate(&desc);
        assert!(
            errors.iter().any(|e| e.contains("dpi_codes is empty")),
            "{errors:?}"
        );
    }
}
