# Spec Delta

## Purpose

Locates a supported Redragon/Holtek mouse and its configuration interface, resolves the matching model descriptor, follows hotplug, and explains access failures in terms the user can act on.

## ADDED Requirements

### Requirement: Supported-device detection

The system SHALL detect a supported mouse by matching each enumerated HID device against the loaded model registry on vendor ID, product ID, HID vendor interface (class 3, subclass 0), and — when the descriptor declares one — a `bcdDevice` range.

Before any feature transfer, the system SHALL parse the interface's report descriptor and confirm the vendor collection declared by the model (usage page and usage) and the declared feature-report IDs and sizes. That collection differs between firmware generations: the attached `04d9:fc5c` exposes `0xFFA0`/`0x01`, while older models use `0xFF00`/`0x01`.

The system MUST list only devices that match a registry entry, and MUST treat the vendor collection interface as the sole configuration channel.

#### Scenario: Descriptor declares a newer vendor page

- **WHEN** the model descriptor declares usage page `0xFFA0` and the device exposes that collection
- **THEN** the device SHALL be accepted for configuration
- **AND** the check SHALL NOT assume `0xFF00`, which belongs to older firmware generations

#### Scenario: Interface lacks the declared collection

- **WHEN** the interface's report descriptor does not contain the collection the descriptor declares
- **THEN** the system SHALL report the device as unsupported for this model and SHALL NOT send any feature report

#### Scenario: Supported mouse attached

- **WHEN** the user starts the application with `04d9:fc5c` attached and a registry entry exists for it
- **THEN** the device status area SHALL show the model name, vendor ID, product ID, `bcdDevice`, and the hidraw node used for configuration
- **AND** the configuration screens SHALL become editable

#### Scenario: Unrelated HID devices present

- **WHEN** the system enumerates keyboards, gamepads, or other HID devices that no registry entry matches
- **THEN** they SHALL NOT be presented as supported devices
- **AND** no feature report SHALL be sent to any of them

#### Scenario: Device present but no registry entry

- **WHEN** a HID device matches the vendor collection usage but its vendor/product ID has no registry entry
- **THEN** the status area SHALL report it as unsupported and display the observed vendor/product ID and `bcdDevice`
- **AND** the system SHALL NOT attempt any read or write against it

### Requirement: Model descriptor resolution

The system MUST resolve exactly one model descriptor for a detected device, and SHALL select the descriptor whose `bcdDevice` range is narrowest when several registry entries share a vendor/product ID.

#### Scenario: Several entries share a product ID

- **WHEN** the registry holds one entry for a product ID with an explicit version range and one without a range
- **THEN** a device inside that range SHALL resolve to the range-specific descriptor
- **AND** a device outside the range SHALL resolve to the unrestricted descriptor

#### Scenario: Conflicting ranges

- **WHEN** two descriptors with explicit ranges both match the device's `bcdDevice`
- **THEN** the system SHALL refuse to resolve the model and report the conflicting descriptor identifiers
- **AND** the system SHALL remain read-only

### Requirement: Vendor-interface access diagnostics

When the configuration interface cannot be opened, the system SHALL report the device node path, the reason (for example permission denied or node absent), the node's current mode and owning account, and the exact udev rule that grants access for that vendor/product ID.

The system MUST NOT elevate privileges, modify system files, or invoke any helper that does.

#### Scenario: Node readable only by root

- **WHEN** the vendor node has mode `0600` and is owned by root while the application runs as an ordinary user
- **THEN** the system SHALL show a permission error naming the node path and displaying a copyable udev rule containing the device's vendor and product ID
- **AND** it SHALL leave the device in read-only state rather than failing silently or exiting

#### Scenario: Device exposes no usable node

- **WHEN** the device is present on the bus but no hidraw node exists for its vendor interface
- **THEN** the system SHALL report that the interface is not exposed and SHALL NOT attempt any feature report transfer

### Requirement: Hotplug handling

The system SHALL detect device arrival and removal without requiring a restart, SHALL refresh device state on arrival, and SHALL retain in-memory edits across a removal.

#### Scenario: Mouse unplugged while editing

- **WHEN** the device disappears while the user has unsaved edits
- **THEN** the edits SHALL be retained and shown as pending
- **AND** controls that require the device SHALL be disabled, with a message stating that the device was disconnected

#### Scenario: Mouse reconnected

- **WHEN** the device is reattached
- **THEN** the system SHALL re-resolve the model and re-read device state
- **AND** it SHALL warn that pending edits were made against a previous connection before allowing them to be applied

### Requirement: Exclusive write access

The system SHALL hold exclusive access to the vendor node for the duration of a write transaction and SHALL refuse to start a transaction while another process holds the node.

#### Scenario: Conflicting writer

- **WHEN** another process holds the vendor node exclusively
- **THEN** the system SHALL present a message identifying the node as busy and suggest retrying after the other process exits
- **AND** it SHALL NOT attempt the write

### Requirement: Stale-state detection

Before applying changes, the system SHALL re-read the registers it is about to modify and SHALL warn the user when their values differ from the values read when the current edits were made.

#### Scenario: Settings changed externally

- **WHEN** the user applies edits and a targeted register now holds a value different from the baseline read
- **THEN** the system SHALL warn that device state changed since it was read, present the differences, and require explicit confirmation before writing
- **AND** cancelling the confirmation SHALL write nothing
