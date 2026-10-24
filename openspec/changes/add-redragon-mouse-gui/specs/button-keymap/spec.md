# Spec Delta

## Purpose

Lets the user decide what each mouse button does — a built-in mouse function, a keyboard key, a multimedia or browser action, a macro, a DPI action, or a multi-key combo — and shows what each button currently does on the device.

## ADDED Requirements

### Requirement: Button inventory

The system SHALL present every button the active model declares, labelled with its physical name, and MUST NOT offer buttons the descriptor does not declare.

#### Scenario: Model with a sniper button

- **WHEN** the model declares a sniper button
- **THEN** that button SHALL appear in the button list with a sniper label
- **AND** it SHALL be assignable like any other declared button

#### Scenario: Fewer buttons on another model

- **WHEN** a descriptor declares seven buttons
- **THEN** exactly those seven SHALL be shown, regardless of what the interface offered for a previously connected model

### Requirement: Assignable action classes

Each button SHALL be assignable to one of: its default function, disabled, a mouse button function, a keyboard key, a multimedia or browser action, a macro, a profile switch target, a DPI cycle or sniper action, or a multi-key combo.

#### Scenario: Assigning a keyboard key

- **WHEN** the user assigns `F5` to button 4
- **THEN** the button's mapping SHALL record the keycode for `F5`
- **AND** applying SHALL write the keymap frame for that button's slot

#### Scenario: Disabling a button

- **WHEN** the user sets button 6 to disabled
- **THEN** the mapping SHALL record the disabled action
- **AND** read-back SHALL show the button as disabled

### Requirement: Multi-key combos

A button MAY hold a combo of several key slots, each with its own keycode and modifier state, and the system MUST reject a combo that exceeds the slot count the descriptor declares or that contains a keycode the model's table does not contain.

#### Scenario: Building a combo

- **WHEN** the user assigns `Ctrl`+`C` to a button
- **THEN** the combo SHALL occupy two key slots with their modifier state recorded
- **AND** applying SHALL write the corresponding keymap and combo frames

#### Scenario: Combo beyond the declared limit

- **WHEN** the user adds a key beyond the declared slot count
- **THEN** the system SHALL refuse the addition and state the limit
- **AND** the existing combo SHALL remain unchanged

#### Scenario: Keycode not in the table

- **WHEN** a combo contains a key the model's keycode table does not encode
- **THEN** the combo SHALL be marked invalid and apply SHALL be blocked

### Requirement: Left-click protection

Reassigning the primary click button MUST require an explicit confirmation that states the user may be unable to click in the application or the desktop afterwards, and the confirmation SHALL name the recovery path.

#### Scenario: Reassigning primary click

- **WHEN** the user assigns a keyboard key to the primary click button
- **THEN** the system SHALL require a confirmation naming the affected button and the risk
- **AND** cancelling SHALL leave the mapping unchanged

#### Scenario: Restoring default click

- **WHEN** the user resets the primary click button to its default
- **THEN** the confirmation SHALL be informational rather than blocking

### Requirement: Current mapping display

The system SHALL read each button's current mapping from the device and display it, including combos and macro bindings, rather than displaying defaults until the user interacts.

#### Scenario: Button bound to a macro

- **WHEN** the device reports button 5 bound to macro `burst-3`
- **THEN** the button list SHALL show that macro name
- **AND** the editor SHALL open with that binding selected

#### Scenario: Unknown encoding read from the device

- **WHEN** a button's register value does not match any action the descriptor can decode
- **THEN** the system SHALL show the raw encoding and mark it as unrecognised
- **AND** it SHALL NOT substitute a default action

### Requirement: Reset a button or profile

The system SHALL let the user reset a single button or every button in the active profile to the descriptor's declared defaults, with confirmation naming the scope.

#### Scenario: Reset all buttons

- **WHEN** the user resets all buttons in the active profile and confirms
- **THEN** the descriptor's default mappings SHALL be written for every button through the transactional path
- **AND** the confirmation SHALL have named the active profile

### Requirement: Keymap write verification

After writing keymap frames, the system SHALL read the affected button slots back and SHALL report any button whose mapping was not accepted.

#### Scenario: Keymap write rejected

- **WHEN** a keymap frame is accepted by the transport but the slot reads back with a different mapping
- **THEN** the system SHALL report that button as not confirmed and keep the pending mapping for retry
