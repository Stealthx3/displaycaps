# EDID corpus

Redacted EDIDs (serial bytes zeroed, serial strings `REDACTED`, checksum fixed) taken from the
committed fixtures, one file per distinct EDID. They seed the fuzz targets and the lab-only oracle
runs (`scripts/oracles.sh corpus`); phase 2 adds the differential tests against edid-decode and
di-edid-decode. libdisplay-info's own test EDIDs are used from its pinned checkout in the lab and are
not committed here. `tests/fixtures/parity/edid/` holds the sink-key parity vectors.
