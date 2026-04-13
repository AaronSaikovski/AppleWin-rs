# Changelog

All notable changes to AppleWin-rs will be documented in this file.

## [Unreleased]

### Added
- Apple IIc (`//c`) model support with embedded ROM (v1.30)
- Model-aware ROM selection: IIc automatically loads its own firmware instead of the IIe Enhanced ROM
- `Bus.model` field for machine-model-aware soft switch behaviour
- `Apple2Model::is_iic()` helper method
- IIc-specific soft switch guards: INTCXROM forced on, SLOTC3ROM writes ignored, DHIRES gated by IOUDIS
- Unit tests for IIc soft switch behaviour
