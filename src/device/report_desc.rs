//! Minimal HID report-descriptor parser.
//!
//! Only the parts this tool needs: the usage page/usage of each top-level
//! application collection, and the size of every feature report. Parsing the
//! real descriptor is what lets the tool refuse a device whose feature reports
//! do not match the model's declared frame sizes.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ReportDescriptor {
    /// Top-level application collections, in descriptor order.
    pub collections: Vec<TopLevelCollection>,
    /// report id (0 for unfragmented) -> feature report size in bits
    pub feature_reports: BTreeMap<u8, u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopLevelCollection {
    pub usage_page: u16,
    pub usage: u16,
}

impl ReportDescriptor {
    pub fn parse(bytes: &[u8]) -> Result<Self, DescriptorParseError> {
        let mut parser = Parser::default();
        let mut index = 0usize;
        while index < bytes.len() {
            let prefix = bytes[index];
            if prefix == 0xFE {
                // Long item: prefix, data size, tag, then data.
                if index + 3 > bytes.len() {
                    return Err(DescriptorParseError::Truncated(index));
                }
                let data_len = bytes[index + 1] as usize;
                index += 3 + data_len;
                continue;
            }
            let size_code = prefix & 0x03;
            let item_type = (prefix >> 2) & 0x03;
            let tag = (prefix >> 4) & 0x0F;
            let data_len = match size_code {
                0 => 0,
                1 => 1,
                2 => 2,
                _ => 4,
            };
            if index + 1 + data_len > bytes.len() {
                return Err(DescriptorParseError::Truncated(index));
            }
            let data = &bytes[index + 1..index + 1 + data_len];
            parser.item(item_type, tag, data)?;
            index += 1 + data_len;
        }
        Ok(parser.finish())
    }

    /// Does this descriptor expose the given top-level collection?
    pub fn has_collection(&self, usage_page: u16, usage: u16) -> bool {
        self.collections
            .iter()
            .any(|c| c.usage_page == usage_page && c.usage == usage)
    }

    /// Feature report size in bytes, rounding up to whole bytes.
    pub fn feature_size_bytes(&self, report_id: u8) -> Option<usize> {
        self.feature_reports
            .get(&report_id)
            .map(|bits| (*bits as usize).div_ceil(8))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DescriptorParseError {
    #[error("report descriptor truncated at byte {0}")]
    Truncated(usize),
    #[error("report descriptor nests collections {0} deep")]
    TooDeep(usize),
}

#[derive(Default)]
struct Parser {
    collections: Vec<TopLevelCollection>,
    feature_reports: BTreeMap<u8, u32>,
    usage_page: u16,
    usages: Vec<u16>,
    report_id: u8,
    report_size: u32,
    report_count: u32,
    depth: usize,
    stack: Vec<Globals>,
}

#[derive(Clone, Copy, Default)]
struct Globals {
    usage_page: u16,
    report_id: u8,
    report_size: u32,
    report_count: u32,
}

impl Parser {
    fn item(&mut self, item_type: u8, tag: u8, data: &[u8]) -> Result<(), DescriptorParseError> {
        let value = unsigned(data);
        match item_type {
            0 => self.main(tag, value)?,
            1 => self.global(tag, value),
            2 => self.local(tag, data, value)?,
            _ => {}
        }
        Ok(())
    }

    fn local(&mut self, tag: u8, data: &[u8], value: u32) -> Result<(), DescriptorParseError> {
        match tag {
            0 => {
                // Usage: a 4-byte form carries an extended usage page.
                if data.len() == 4 {
                    let page = (value >> 16) as u16;
                    let usage = (value & 0xFFFF) as u16;
                    self.usages.push(usage);
                    if page != 0 && page != self.usage_page {
                        // Extended usage overrides the page for this usage only;
                        // remember it so a collection can record it.
                        self.usage_page = page;
                    }
                } else {
                    self.usages.push(value as u16);
                }
            }
            1 | 2 => {}
            _ => {}
        }
        Ok(())
    }

    fn global(&mut self, tag: u8, value: u32) {
        match tag {
            0 => self.usage_page = value as u16,
            7 => self.report_size = value,
            8 => self.report_id = value as u8,
            9 => self.report_count = value,
            10 => self.stack.push(Globals {
                usage_page: self.usage_page,
                report_id: self.report_id,
                report_size: self.report_size,
                report_count: self.report_count,
            }),
            11 => {
                if let Some(globals) = self.stack.pop() {
                    self.usage_page = globals.usage_page;
                    self.report_id = globals.report_id;
                    self.report_size = globals.report_size;
                    self.report_count = globals.report_count;
                }
            }
            _ => {}
        }
    }

    fn main(&mut self, tag: u8, value: u32) -> Result<(), DescriptorParseError> {
        match tag {
            0x8 => {
                // Input
            }
            0x9 => {
                // Output
            }
            0xB => {
                let bits = self.report_size.saturating_mul(self.report_count);
                // Report IDs are per interface: if two collections declare the
                // same ID, the larger definition wins rather than adding up.
                let entry = self.feature_reports.entry(self.report_id).or_insert(0);
                *entry = (*entry).max(bits);
            }
            0xA => {
                // Collection: the data byte is the collection type.
                let collection_type = value & 0xFF;
                if self.depth == 0 && collection_type == 0x01 {
                    let usage = self.usages.last().copied().unwrap_or(0);
                    self.collections.push(TopLevelCollection {
                        usage_page: self.usage_page,
                        usage,
                    });
                }
                self.depth += 1;
                if self.depth > 8 {
                    return Err(DescriptorParseError::TooDeep(self.depth));
                }
            }
            0xC => {
                // End Collection
                self.depth = self.depth.saturating_sub(1);
            }
            _ => {}
        }
        self.usages.clear();
        Ok(())
    }

    fn finish(self) -> ReportDescriptor {
        ReportDescriptor {
            collections: self.collections,
            feature_reports: self.feature_reports,
        }
    }
}

fn unsigned(data: &[u8]) -> u32 {
    let mut value = 0u32;
    for (i, byte) in data.iter().take(4).enumerate() {
        value |= (*byte as u32) << (8 * i);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic descriptor with a consumer collection and a vendor collection
    /// carrying 16-, 64- and 1024-byte feature reports.
    fn sample() -> Vec<u8> {
        vec![
            // Usage Page (Consumer) 0x0C
            0x05, 0x0C, // Usage (Consumer Control) 0x01
            0x09, 0x01, // Collection (Application)
            0xA1, 0x01, //   Report ID 1: 15 bytes
            0x85, 0x01, 0x75, 0x08, 0x95, 0x0F, 0xB1, 0x02, // End Collection
            0xC0, // Usage Page (Vendor 0xFF00)
            0x06, 0x00, 0xFF, // Usage (0x01)
            0x09, 0x01, // Collection (Application)
            0xA1, 0x01, //   Report ID 2: 15 bytes
            0x85, 0x02, 0x75, 0x08, 0x95, 0x0F, 0xB1, 0x02, //   Report ID 3: 63 bytes
            0x85, 0x03, 0x75, 0x08, 0x95, 0x3F, 0xB1, 0x02, //   Report ID 4: 1023 bytes
            0x85, 0x04, 0x75, 0x08, 0x96, 0xFF, 0x03, 0xB1, 0x02, // End Collection
            0xC0,
        ]
    }

    #[test]
    fn parses_top_level_collections() {
        let parsed = ReportDescriptor::parse(&sample()).expect("parse");
        assert!(parsed.has_collection(0x0C, 0x01));
        assert!(parsed.has_collection(0xFF00, 0x01));
        assert_eq!(parsed.collections.len(), 2);
    }

    #[test]
    fn sums_feature_report_sizes() {
        let parsed = ReportDescriptor::parse(&sample()).expect("parse");
        // Sizes exclude the report-ID byte that hidraw prepends.
        assert_eq!(parsed.feature_size_bytes(2), Some(15));
        assert_eq!(parsed.feature_size_bytes(3), Some(63));
        assert_eq!(parsed.feature_size_bytes(4), Some(1023));
    }

    #[test]
    fn truncated_input_is_rejected() {
        let err = ReportDescriptor::parse(&[0x06, 0x00]).expect_err("must fail");
        assert!(matches!(err, DescriptorParseError::Truncated(_)));
    }
}
