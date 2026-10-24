# Spec Delta

## Purpose

Defines the behaviour of the desktop application's interface: how device status is presented, how edits are reviewed before they reach the hardware, how failures are surfaced, and how the tool is driven without a graphical session.

## ADDED Requirements

### Requirement: Single-window sectioned layout

The application SHALL present a single window with a persistent device status header and a navigable set of sections covering device information, pointer tuning, lighting, profiles, buttons, macros, backups, and diagnostics.

#### Scenario: Start with a supported device

- **WHEN** the application starts and a supported, accessible device is present
- **THEN** the status header SHALL show the model, IDs, connection state, and whether pending edits exist
- **AND** all sections the model supports SHALL be reachable

#### Scenario: Start with no device

- **WHEN** the application starts and no supported device is present
- **THEN** the application SHALL remain open, show a clear no-device state with the access guidance where relevant, and keep retrying detection
- **AND** it SHALL NOT exit with an error

### Requirement: Dirty-state visibility and review

The interface SHALL show which settings differ from the device's state, SHALL summarise the pending change set before applying, and SHALL enable apply only when at least one setting differs.

#### Scenario: Reviewing a change set

- **WHEN** the user has pending edits and requests apply
- **THEN** the interface SHALL list the changed settings with their before and after values
- **AND** apply SHALL proceed only after the user confirms that summary

#### Scenario: Nothing changed

- **WHEN** no setting differs from the device state
- **THEN** apply SHALL be disabled

### Requirement: Error surfacing

Every failure SHALL be presented in the interface with a specific cause and a suggested next step, and SHALL also be written to a session log file whose location is shown in the diagnostics section.

#### Scenario: Device error during use

- **WHEN** an operation fails because the device disappeared
- **THEN** the interface SHALL show the cause, the operation attempted, and the suggestion to reconnect
- **AND** the same record SHALL be in the session log

#### Scenario: Log location discoverable

- **WHEN** the user opens diagnostics
- **THEN** the current log file path SHALL be displayed and selectable for copying

### Requirement: Destructive-action confirmation

The application SHALL require confirmation before applying changes to the device, restoring a backup, resetting a profile or button set, deleting a macro, reassigning the primary click button, and overwriting an export file.

#### Scenario: Confirmation names the effect

- **WHEN** any destructive action is requested
- **THEN** the confirmation SHALL name the action, the affected profile or asset, and the consequence
- **AND** the default focused control SHALL be the non-destructive choice

### Requirement: Keyboard operation

Every interactive control SHALL be reachable and operable by keyboard, the current focus SHALL be visually identifiable, and dialogs SHALL trap focus until dismissed with a cancel path always available.

#### Scenario: Applying without a pointer

- **WHEN** the user navigates to apply using only the keyboard and confirms
- **THEN** the change set summary SHALL be reachable by keyboard and the apply SHALL complete

#### Scenario: Dismissing a dialog

- **WHEN** a confirmation dialog has focus and the user presses the cancel shortcut
- **THEN** the dialog SHALL close without performing the action

### Requirement: Theme and scaling

The interface SHALL follow the desktop's light or dark preference and SHALL respect the desktop's text scaling without clipping required content.

#### Scenario: Dark preference at large scale

- **WHEN** the desktop reports a dark preference and 150% text scaling
- **THEN** the interface SHALL render in its dark variant with no clipped labels or controls

### Requirement: No privilege escalation

The application MUST NOT request or require elevated privileges, MUST NOT install system files on its own, and SHALL function with only the access the user has been granted for the vendor node.

#### Scenario: Started without access

- **WHEN** the user starts the application without read access to the vendor node
- **THEN** the application SHALL run, report the permission problem with the udev rule, and offer no elevated retry

### Requirement: Headless diagnostics

The application SHALL offer non-interactive flags that report supported-device detection and, when access allows, the current register state as text, exiting with a distinct status code for success, no supported device, and access failure.

#### Scenario: Listing devices without a display

- **WHEN** the application is invoked with its device-listing flag on a machine with no display server
- **THEN** it SHALL print the detected supported devices and exit with the success status code

#### Scenario: Access failure status

- **WHEN** the state-dump flag is used while the vendor node is unreadable
- **THEN** it SHALL print the access guidance and exit with the access-failure status code

### Requirement: Hardware-free validation mode

The application SHALL support running its interface and protocol encoding against a simulated device, so behaviour can be exercised and tested without hardware attached.

#### Scenario: Simulated device

- **WHEN** the application runs in simulation mode with a described model
- **THEN** all sections SHALL be operable and apply SHALL exercise the full validate/apply/verify path against the simulation
- **AND** no hardware access SHALL occur
