# Spec Delta

## Purpose

Gives the user control over the mouse's lighting — effect mode, colour, brightness, effect speed, and zones where the model has them — and degrades honestly on models whose lighting cannot be configured.

## ADDED Requirements

### Requirement: Effect mode selection

The system SHALL present only the lighting effects the active model declares — such as static, breathing, rainbow, reactive, wave, flash, and off — and SHALL write the mode encoding for the selected effect to the active profile.

#### Scenario: Selecting a declared effect

- **WHEN** the user selects breathing on a model that declares it and applies
- **THEN** the mode encoding for breathing SHALL be written to the active profile's lighting register
- **AND** the interface SHALL show the effect name read back from the device

#### Scenario: Undeclared effect absent

- **WHEN** the model does not declare the wave effect
- **THEN** wave SHALL NOT appear in the effect list

### Requirement: Colour selection

Where the selected effect uses a single colour, the system SHALL let the user choose it as RGB, HSV, or hex, SHALL keep the three representations synchronised, and SHALL write the resulting colour bytes in the order the descriptor declares.

#### Scenario: Hex entry

- **WHEN** the user enters `#FF8800`
- **THEN** the RGB and HSV controls SHALL show the same colour
- **AND** applying SHALL write red `0xFF`, green `0x88`, blue `0x00` in the declared byte order

#### Scenario: Effect ignores colour

- **WHEN** the selected effect does not use a colour, such as rainbow
- **THEN** the colour controls SHALL be shown disabled with the reason stated
- **AND** colour bytes SHALL NOT be written for that apply

### Requirement: Colour persistence across effect changes

Changing effect SHALL preserve the user's last chosen colour so that switching to a colour-using effect restores it, and applying an effect that ignores colour MUST NOT overwrite the stored colour bytes.

#### Scenario: Round trip through rainbow

- **WHEN** the user sets a colour, switches to rainbow, applies, then switches back to static
- **THEN** the previously chosen colour SHALL still be selected

### Requirement: Brightness and effect speed

The system SHALL expose brightness and effect speed at the levels the descriptor declares, SHALL show the device's current level after connecting, and SHALL reject out-of-range levels before applying.

#### Scenario: Brightness change

- **WHEN** the user lowers the brightness one level and applies
- **THEN** the brightness register SHALL be written with the new level
- **AND** read-back SHALL confirm it

#### Scenario: Speed not declared

- **WHEN** the selected effect has no adjustable speed on this model
- **THEN** the speed control SHALL be disabled and no speed register SHALL be written

### Requirement: Zone control

Where the descriptor declares lighting zones, the system SHALL let the user address each declared zone and an all-zones selection, and SHALL hide zones the model does not have.

#### Scenario: Front zone only

- **WHEN** the user selects the front zone and applies a colour
- **THEN** only that zone's register SHALL be written
- **AND** other zones SHALL remain unchanged

#### Scenario: Model without zones

- **WHEN** the descriptor declares no zones
- **THEN** no zone selector SHALL be shown and the all-zones path SHALL be used

### Requirement: Honest unsupported state

On a model whose descriptor does not declare configurable lighting, the system SHALL present the lighting screen as unsupported with an explanation, and MUST NOT write any lighting register.

#### Scenario: Non-RGB model connected

- **WHEN** the active model has no configurable lighting
- **THEN** the lighting screen SHALL state that the model does not expose lighting configuration
- **AND** no lighting feature report SHALL be sent

### Requirement: Preview is not device state

Any colour or effect preview rendered by the interface SHALL be distinguishable from the value read from the device, and the interface MUST NOT present an unapplied preview as the device's current setting.

#### Scenario: Preview before apply

- **WHEN** the user adjusts colour without applying
- **THEN** the device-state indicator SHALL continue to show the read-back value
- **AND** the pending value SHALL be marked as not yet on the device
