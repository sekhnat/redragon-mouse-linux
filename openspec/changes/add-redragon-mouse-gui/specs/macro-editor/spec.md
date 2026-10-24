# Spec Delta

## Purpose

Lets the user author the input sequences a button plays back — multi-action macros with key presses and delays, and the burst-fire editor — with limits that reflect what each model can store.

## ADDED Requirements

### Requirement: Macro library

The system SHALL present the macro slots the active model declares, SHALL let the user create, rename, edit, duplicate, and delete a macro within those slots, and MUST refuse to create more macros than the declared slot count.

#### Scenario: Creating a macro

- **WHEN** the user creates a macro named `reload-combo` in a free slot
- **THEN** the macro SHALL appear in the library and be assignable to a button
- **AND** it SHALL exist only as a pending edit until applied

#### Scenario: Slot limit reached

- **WHEN** every declared macro slot is occupied and the user creates another
- **THEN** the system SHALL refuse and identify the limit
- **AND** no existing macro SHALL be modified

#### Scenario: Deleting a bound macro

- **WHEN** the user deletes a macro that a button is bound to
- **THEN** the system SHALL warn that the button binding will no longer resolve and require confirmation
- **AND** the affected button SHALL be shown as needing a new binding before apply

### Requirement: Macro actions and ordering

A macro SHALL be an ordered list of actions supporting at least key down, key up, mouse button down and up, and delay, each with a duration where applicable, and the editor SHALL reject an entry that cannot be encoded by the model's action format.

#### Scenario: Building a sequence

- **WHEN** the user adds key down `W`, a 40 ms delay, and key up `W`
- **THEN** the editor SHALL show the three actions in order with the delay value
- **AND** applying SHALL write them as consecutive action records

#### Scenario: Unencodable entry

- **WHEN** an action's keycode or delay cannot be represented in the model's record format
- **THEN** the entry SHALL be rejected with the reason stated
- **AND** the rest of the macro SHALL remain editable

### Requirement: Macro size limits

The system MUST enforce the per-button macro storage limit declared by the descriptor and SHALL block apply when a macro exceeds it, showing how much of the limit is used.

#### Scenario: Oversized macro

- **WHEN** a macro's encoded record length exceeds the declared per-button limit
- **THEN** apply SHALL be blocked and the overflow amount SHALL be shown
- **AND** no macro frame SHALL be sent

### Requirement: Burst-fire editor

The system SHALL provide a burst-fire editor for the button function that repeats clicks, exposing the bullet count and the interval, bounded by the descriptor's declared ranges, and SHALL record the values with the button binding.

#### Scenario: Configuring burst fire

- **WHEN** the user sets three bullets at the minimum interval and binds them to button 4
- **THEN** the button mapping SHALL record the burst-fire action with those values
- **AND** read-back SHALL show the same values

#### Scenario: Out-of-range bullet count

- **WHEN** the user enters a bullet count above the declared maximum
- **THEN** the value SHALL be rejected with the accepted range shown
- **AND** apply SHALL remain blocked until corrected

### Requirement: Verified macro writes

Writing a macro SHALL use the report layout the descriptor declares for the bound button, SHALL read the stored record back, and SHALL report a mismatch as a failure rather than success.

#### Scenario: Macro read-back mismatch

- **WHEN** the stored macro record differs from what was written
- **THEN** the system SHALL report the macro as not confirmed, identify the bound button, and offer a retry
- **AND** it SHALL NOT clear the pending macro state

#### Scenario: Macro written and confirmed

- **WHEN** the stored record matches the intended record
- **THEN** the macro SHALL be marked as on the device
- **AND** the backup taken before the write SHALL be retained

### Requirement: Macro import and export

The system SHALL export macros to a portable file with the action list, delays, name, and binding, and SHALL import such files with full validation against the active model before anything is applied.

#### Scenario: Export and re-import

- **WHEN** a macro is exported and the file is imported into the same model
- **THEN** the imported macro SHALL have the same actions, delays, and name
- **AND** it SHALL require a free slot and an explicit apply to reach the device

#### Scenario: Import into a model that cannot encode it

- **WHEN** an imported macro contains an action the active model cannot encode
- **THEN** the import SHALL fail naming the unsupported action
- **AND** the existing library SHALL remain unchanged

### Requirement: Live editing without device writes

Editing a macro MUST NOT send any frame until the user applies, and discarding pending macro edits SHALL restore the previously read macro content exactly.

#### Scenario: Discard macro edits

- **WHEN** the user edits a macro and then reverts
- **THEN** the macro SHALL return to the content read from the device
- **AND** no feature report SHALL have been sent
