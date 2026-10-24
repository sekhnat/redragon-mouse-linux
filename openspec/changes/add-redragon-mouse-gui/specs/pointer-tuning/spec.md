# Spec Delta

## Purpose

Lets the user tune how the pointer behaves — sensitivity stages, polling rate, angle snap, lift-off distance, scroll speed, acceleration, and double-click speed — with every offered value bounded by what the connected model actually supports.

## ADDED Requirements

### Requirement: Five DPI stages

Each profile SHALL expose its five DPI stages, and each stage SHALL be individually enabled or disabled, SHALL be settable to X/Y-linked or independent per-axis DPI, and SHALL accept only values present in the model's DPI code table.

#### Scenario: Independent axes

- **WHEN** the user unlinks the axes of stage 2 and sets X to 800 and Y to 1600
- **THEN** the interface SHALL show both values independently
- **AND** applying SHALL write both axis values for that stage

#### Scenario: Linked axes

- **WHEN** the axes are linked and the user sets 1600
- **THEN** both X and Y SHALL read 1600
- **AND** changing either SHALL change the other

#### Scenario: Value not in the code table

- **WHEN** the user enters a DPI value the model's table does not encode
- **THEN** the value SHALL be rejected with the nearest supported values shown
- **AND** the stage SHALL keep its previous value

### Requirement: DPI stage invariants

At least one stage MUST remain enabled, the active stage MUST be one of the enabled stages, and disabling the active stage SHALL move the active stage to the nearest enabled stage.

#### Scenario: Disabling the active stage

- **WHEN** the user disables the stage that is currently active
- **THEN** the active stage SHALL move to the nearest enabled stage
- **AND** the interface SHALL make the new active stage visible before apply

#### Scenario: Disabling the last enabled stage

- **WHEN** only one stage is enabled and the user tries to disable it
- **THEN** the system SHALL refuse and explain that at least one stage is required

### Requirement: Stage switching and sniper behaviour

The profile SHALL define the DPI cycle order used by the DPI-switch button and SHALL allow the sniper button to target a chosen DPI stage or DPI value, both limited to enabled stages.

#### Scenario: Sniper mapped to a slower stage

- **WHEN** the user sets the sniper action to stage 1
- **THEN** the profile SHALL record stage 1 as the sniper target
- **AND** the selection SHALL be listed among the button's assignable DPI actions

#### Scenario: Sniper targeting a disabled stage

- **WHEN** the user selects a sniper target that is not an enabled stage
- **THEN** the system SHALL refuse the selection and state that the target must be enabled

### Requirement: Polling rate

The system SHALL present the polling rates declared by the model — 125, 250, 500, and 1000 Hz on models that declare all four — per profile, and SHALL write the corresponding register encoding for the selected rate.

#### Scenario: Changing polling rate

- **WHEN** the user selects 1000 Hz for profile 3 and applies
- **THEN** the register encoding for 1000 Hz SHALL be written to profile 3's polling-rate slot
- **AND** read-back SHALL confirm the new encoding

#### Scenario: Rate not supported by the model

- **WHEN** the model does not declare 250 Hz
- **THEN** that option SHALL NOT be presented at all

### Requirement: Angle snap and lift-off distance

Where the descriptor declares them, the system SHALL expose angle snap as a toggle and lift-off distance as a choice among the values the model declares (1, 2, or 3 mm), and these device-level settings SHALL be applied independently of the active profile.

#### Scenario: Changing lift-off distance

- **WHEN** the user selects 2 mm and applies while profile 1 is active
- **THEN** the device-level lift-off register SHALL be written
- **AND** no profile-specific register SHALL be written for that change

#### Scenario: Model without lift-off control

- **WHEN** the descriptor does not declare lift-off distance
- **THEN** the control SHALL be hidden or shown disabled with a stated reason
- **AND** the corresponding register SHALL NOT be written

### Requirement: Scroll, acceleration, and double-click settings

The system SHALL expose scroll speed, scroll-in-page toggle, acceleration enable, and double-click speed with the ranges declared by the descriptor, and SHALL reject values outside those ranges before applying.

#### Scenario: Out-of-range double-click speed

- **WHEN** the user sets a double-click speed above the declared maximum
- **THEN** apply SHALL be blocked with the field marked invalid
- **AND** no register SHALL be written

#### Scenario: Scroll speed change

- **WHEN** the user raises the scroll speed and applies
- **THEN** only the scroll-speed register SHALL be written

### Requirement: Values round-trip through the device

For every setting in this capability, the value shown after a successful apply SHALL equal the value read back from the device, and the value shown after connecting SHALL equal the device's stored setting, including values the model clamps or normalises.

#### Scenario: Device normalises a value

- **WHEN** the device stores a different code than the one requested, and the read-back maps to a different supported value
- **THEN** the interface SHALL display the read-back value and mark the original entry as adjusted
- **AND** it SHALL NOT continue to show the unaccepted value as applied
