# evtap roadmap

This roadmap describes possible directions rather than a release plan or compatibility promise. evtap follows Cargo's pre-1.0 semantic-versioning rules: incompatible changes may ship in a new minor release (`0.1` to `0.2`), while patch releases should remain compatible within their release line.

## Potential future directions

- Longitudinal trends, distributions, and confidence indicators
- Physical keyboard geometry and layout-aware heatmap visualization
- Better automatic desktop XKB configuration detection
- Device hotplug monitoring and smoother reconnect behavior
- More robust correction and editing analysis with explicit uncertainty
- Optional aggregate export designed around a stable data schema
- A deliberately public library or plugin API if external consumers emerge
- Additional operating systems when their capture and permission models can be supported responsibly

Raw keystroke persistence, cloud synchronization, accounts, and telemetry are explicit non-goals. Reconsidering any of them requires a separate threat model and explicit product decision.
