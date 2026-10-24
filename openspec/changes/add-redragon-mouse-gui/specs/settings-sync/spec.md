# Spec Delta

## Purpose

Turns raw feature-report reads into a validated, editable device state and applies user edits to the hardware as an explicit, verified, reversible transaction with a backup of what was there before.

## ADDED Requirements

### Requirement: Read current state before editing

On connecting to a device, the system SHALL read every register declared by the active descriptor before presenting editable controls, and SHALL populate those controls from the values read.

If any required read fails, the system MUST present the device as read-only and report which reads failed; it MUST NOT present defaults or cached values as if they were read from the device.

#### Scenario: Successful read-back

- **WHEN** the device is readable and the user opens any settings screen
- **THEN** every shown value SHALL originate from a feature report read in this session
- **AND** the read values SHALL be retained as the baseline for revert and change detection

#### Scenario: Partial read failure

- **WHEN** reading the keymap registers fails while DPI registers read successfully
- **THEN** the keymap screen SHALL be shown as unavailable with the failure reason
- **AND** the system SHALL NOT allow applying changes that depend on the failed reads

### Requirement: Validation before any write

The system SHALL validate the complete pending state against the active descriptor — enumerated values, address bounds, and per-field ranges — before sending the first frame.

If any value is invalid, the system MUST send nothing and SHALL identify each invalid field.

#### Scenario: Out-of-range pending value

- **WHEN** the pending state contains a polling rate the descriptor does not declare
- **THEN** apply SHALL be blocked, the offending field SHALL be highlighted, and no feature report SHALL be sent

#### Scenario: Entirely valid pending state

- **WHEN** every pending value validates
- **THEN** apply SHALL proceed to the confirmation step

### Requirement: Transactional apply

Applying changes SHALL be a single transaction: begin framing for the target profile, one frame per changed register, then commit framing.

On failure mid-transaction, the system SHALL attempt to restore the registers already written in that transaction and SHALL report the registers that could not be restored.

#### Scenario: Failure during apply

- **WHEN** a register write fails after earlier registers were written
- **THEN** the system SHALL attempt to rewrite the previously written registers with their pre-transaction values
- **AND** it SHALL report the failure, the frames attempted, and the outcome of the rollback attempt

#### Scenario: Successful apply

- **WHEN** every frame is accepted
- **THEN** the system SHALL send the commit frame
- **AND** it SHALL report the number of registers changed

### Requirement: Write verification

After applying, the system SHALL read back every register it wrote and compare the result with the intended state, and SHALL report each mismatch rather than declaring success.

#### Scenario: Read-back mismatch

- **WHEN** a written register reads back with a different value
- **THEN** the system SHALL report that register as not confirmed and keep the pending state marked as applied-with-warnings
- **AND** it SHALL offer to retry the affected registers

#### Scenario: Read-back matches

- **WHEN** every written register reads back as intended
- **THEN** the system SHALL confirm the apply, clear the dirty state, and update the baseline to the read-back values

### Requirement: Change-set minimization

The system MUST send frames only for registers whose value differs from the baseline, and SHALL NOT rewrite unchanged registers.

#### Scenario: Single setting changed

- **WHEN** the user changes only the LED brightness and applies
- **THEN** the transaction SHALL contain frames for the brightness register and the required begin/commit framing only
- **AND** DPI, polling rate, and keymap registers SHALL receive no frames

### Requirement: Automatic backup and restore

Before the first frame of every successful validation, the system SHALL write a timestamped backup of the pre-apply register values, the descriptor identifier, the device IDs, and the active profile.

The system SHALL allow restoring a backup, SHALL verify a restore by read-back, and SHALL keep backups until the user deletes them.

#### Scenario: Backup written on apply

- **WHEN** an apply passes validation and the user confirms
- **THEN** a backup file SHALL exist before any frame is sent
- **AND** its contents SHALL be sufficient to restore the prior state

#### Scenario: Restore a backup

- **WHEN** the user selects a backup and confirms restore
- **THEN** the system SHALL write the recorded values through the same transactional and verification path
- **AND** it SHALL require confirmation that names the backup timestamp and the affected profile

### Requirement: Explicit apply and revert

Edits SHALL remain pending until the user applies them, and a revert action SHALL discard pending edits and restore the baseline values in the interface.

Closing the application with pending edits MUST warn that the edits are unapplied.

#### Scenario: Close with pending edits

- **WHEN** the user quits while edits are pending
- **THEN** the system SHALL offer to apply, discard, or cancel the quit
- **AND** choosing discard SHALL write nothing to the device

#### Scenario: Revert pending edits

- **WHEN** the user reverts
- **THEN** every control SHALL return to the baseline value
- **AND** no feature report SHALL be sent

### Requirement: Write failure reporting

Every transport failure SHALL be reported with the operation attempted, the register or frame involved, and the underlying error, and SHALL be written to the session log.

#### Scenario: Transport error

- **WHEN** a feature report write returns an error
- **THEN** the interface SHALL show which register failed and the OS error text
- **AND** the session log SHALL contain the same detail
