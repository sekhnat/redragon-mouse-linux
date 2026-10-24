# Spec Delta

## Purpose

Defines the per-model data that describes a mouse's register layout and enumerations, so that device support is added as reviewed data rather than as new code, with provenance and confidence recorded alongside it.

## ADDED Requirements

### Requirement: Descriptor completeness

Every model descriptor SHALL declare the register layout needed to read and write supported features: profile begin/commit framing, register addresses and per-profile strides for DPI stages, polling rate, LED mode/colour/brightness, scroll speed, keymap, and macro storage, plus the enumerations for DPI codes, keycodes, LED modes, LED zones, and the button inventory.

Descriptors that omit a section required by an enabled feature MUST be rejected.

#### Scenario: Incomplete descriptor

- **WHEN** a descriptor lacks the DPI address stride required by the DPI feature
- **THEN** loading SHALL fail for that descriptor and report the missing field
- **AND** the remaining descriptors SHALL continue to load

#### Scenario: Complete descriptor

- **WHEN** a descriptor declares every section for the attached model
- **THEN** all features it declares SHALL be available in the interface
- **AND** features it does not declare SHALL be absent rather than displayed as broken controls

### Requirement: Capability declaration

A descriptor SHALL declare which optional capabilities the model supports — RGB lighting, lighting zones, angle snap, lift-off distance, per-axis DPI, polling rates, macro slot count, and button inventory — and the system SHALL treat undeclared capabilities as unsupported.

#### Scenario: Non-RGB model

- **WHEN** a descriptor omits RGB lighting
- **THEN** the lighting capability SHALL be reported as unsupported for that model
- **AND** no lighting register SHALL be written

#### Scenario: Reduced polling-rate set

- **WHEN** a descriptor declares only 125, 500 and 1000 Hz
- **THEN** only those rates SHALL be offered
- **AND** 250 Hz SHALL NOT be selectable

### Requirement: Provenance and confidence

Every descriptor SHALL record where its data came from (register capture from vendor software, derivation from an upstream open-source implementation, or live read-back from the device) and a verification state of `verified` or `unverified`.

The system MUST surface the verification state for the active model and MUST require an additional confirmation before writing through an `unverified` descriptor.

#### Scenario: Unverified descriptor

- **WHEN** the active model's descriptor is marked `unverified`
- **THEN** the status area SHALL show a persistent warning naming the provenance
- **AND** the first write in a session SHALL require an extra confirmation that explains the risk

#### Scenario: Verified descriptor

- **WHEN** the active model's descriptor is marked `verified`
- **THEN** no extra confirmation SHALL be required beyond the normal apply confirmation
- **AND** the provenance SHALL still be viewable in the model info panel

### Requirement: Adding a model without code changes

The system SHALL discover model descriptors from its data directory at startup, and adding support for a new model MUST require only a new descriptor file plus its registry entry, with no rebuild.

#### Scenario: New descriptor dropped in

- **WHEN** a valid descriptor and registry entry for a new product ID are placed in the data directory
- **THEN** the next launch SHALL offer that model as supported
- **AND** all of its declared features SHALL be usable without recompiling

#### Scenario: Malformed descriptor present

- **WHEN** a descriptor fails parsing
- **THEN** the system SHALL report the file and the parse error, SHALL skip that descriptor, and SHALL stay fully functional for other models

### Requirement: Descriptor schema versioning

Each descriptor SHALL declare a schema version, and the system MUST reject descriptors whose major version it does not support while continuing to load compatible ones.

#### Scenario: Future schema version

- **WHEN** a descriptor declares an unsupported major schema version
- **THEN** the system SHALL report the required and supported versions
- **AND** SHALL NOT use the descriptor for any transfer

### Requirement: Enumerated values are exhaustive

Every value the interface offers for an enumerated setting SHALL exist in the descriptor's table, and the system MUST NOT interpolate, extrapolate, or guess a value that the table does not contain.

#### Scenario: DPI value absent from the table

- **WHEN** the user requests a DPI value that the model's code table does not contain
- **THEN** the system SHALL reject the value and show the nearest values that do exist
- **AND** it SHALL NOT compute an approximate register code

### Requirement: Descriptor data is inspectable

The system SHALL expose the active descriptor — identifier, provenance, verification state, and declared capabilities — to the user, and SHALL include the descriptor identifier and schema version in every backup and log record it writes.

#### Scenario: Backup records descriptor

- **WHEN** a backup of device state is written
- **THEN** it SHALL contain the descriptor identifier and schema version used to read that state
