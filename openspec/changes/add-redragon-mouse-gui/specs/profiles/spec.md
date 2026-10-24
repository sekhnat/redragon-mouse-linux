# Spec Delta

## Purpose

Manages the mouse's five onboard profile slots — which one is active, what each one contains, and how settings move between the device and portable profile files.

## ADDED Requirements

### Requirement: Onboard profile inventory

The system SHALL present the onboard profile slots declared by the model (five on the current models) plus each slot's name, and SHALL show which slot is currently active.

#### Scenario: Connecting to a device

- **WHEN** the device connects and the active profile is read
- **THEN** the interface SHALL show all five slots and mark the active one
- **AND** it SHALL read the settings of the active slot before editing is enabled

#### Scenario: Model with fewer slots

- **WHEN** a descriptor declares three slots
- **THEN** only three slots SHALL be shown and addressable

### Requirement: Profile switching

The system SHALL let the user activate a different slot, SHALL apply the activation as its own confirmed device action using the profile begin and commit framing, and SHALL update the displayed active slot only after read-back confirms it.

#### Scenario: Activating another slot

- **WHEN** the user activates slot 3
- **THEN** the system SHALL send the activation sequence and read the active profile back
- **AND** the interface SHALL mark slot 3 as active only after the read-back matches

#### Scenario: Activation not confirmed

- **WHEN** the read-back after activation still reports the previous slot
- **THEN** the system SHALL report that the switch was not confirmed and leave the previously active slot marked

#### Scenario: Switching with pending edits

- **WHEN** the user activates another slot while edits are pending
- **THEN** the system SHALL offer to apply or discard those edits first
- **AND** it SHALL NOT switch while edits are left unresolved

### Requirement: Editing a profile in isolation

Applying changes for one slot MUST NOT modify the settings of other slots, and after an apply the system SHALL verify that the other slots' registers are unchanged.

#### Scenario: Editing slot 2

- **WHEN** the user changes slot 2's DPI and applies
- **THEN** slot 2's DPI registers SHALL be written
- **AND** slots 1, 3, 4, and 5 SHALL read back unchanged

#### Scenario: Unexpected change in another slot

- **WHEN** read-back shows another slot's register changed
- **THEN** the system SHALL report which slot and register changed and offer the pre-apply backup for restore

### Requirement: Copy and reset

The system SHALL let the user copy one slot's settings onto another and reset a slot to the descriptor's declared defaults, both with confirmation naming the source and destination slots.

#### Scenario: Copy slot 1 to slot 4

- **WHEN** the user copies slot 1 onto slot 4 and confirms
- **THEN** slot 4's registers SHALL be written with slot 1's settings through the transactional path
- **AND** the interface SHALL show slot 4's new values after read-back

#### Scenario: Reset a slot

- **WHEN** the user resets slot 5 to defaults and confirms
- **THEN** the descriptor's default values SHALL be written for that slot
- **AND** a backup of the prior slot contents SHALL exist

### Requirement: Profile file import and export

The system SHALL export a slot to an INI profile file using the field names of the vendor profile format and SHALL import such files, accepting the documented schema variants (`KeyInfo_i_` and `KeyInfo_i_L` forms, `acee` and `aceel`).

Import MUST validate every imported value against the active descriptor and MUST reject the file as a whole when any value is unsupported, listing each offending key.

#### Scenario: Exporting a slot

- **WHEN** the user exports slot 2
- **THEN** a file SHALL be written containing the slot's polling rate, DPI stages, button mappings with key slots, and the remaining profile fields using the vendor field names

#### Scenario: Importing a vendor file

- **WHEN** the user imports a profile file taken from the vendor data tree
- **THEN** all supported values SHALL populate the pending profile
- **AND** any field the active descriptor does not recognise SHALL be reported before applying

#### Scenario: Import with an unsupported value

- **WHEN** an imported file contains a polling rate the model does not declare
- **THEN** the import SHALL fail with the offending key and value named
- **AND** the current pending profile SHALL remain unchanged

### Requirement: Round-trip fidelity

Exporting a slot and importing that file back MUST produce the same pending profile values, and the file SHALL remain readable by the vendor application for the fields it shares.

#### Scenario: Export then import

- **WHEN** a slot is exported and the resulting file is imported into the same model
- **THEN** every value in the pending profile SHALL equal the exported slot's values
- **AND** no field SHALL be reported as unrecognised

### Requirement: Unsaved profile files

Before overwriting or replacing an existing profile file, the system SHALL confirm and SHALL keep the previous file content recoverable.

#### Scenario: Overwriting an export target

- **WHEN** the user exports to a path that already exists and confirms
- **THEN** the system SHALL replace the file and preserve the prior content as a recoverable copy
