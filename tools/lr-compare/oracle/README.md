# Camera Raw oracle (Bryan's fork)

Measure Lightroom's PV2012 rendering exactly: synthetic LinearRaw DNGs (`dng.py`, `scenes.py`; real
camera colour from the installed Adobe Standard DCPs via `dcp_info`) carrying Camera Raw settings in
their XMP, rendered by Camera Raw in Photoshop (`ps.sh` + `batch.jsx`, 16-bit ProPhoto TIFFs) and by
LightCraft (`cargo run --release -p lightcraft-engine --example oracle_render`), compared pixel by
pixel. `make_set*.py` build the experiments (slider sweeps on ramps, probes on backgrounds,
flats / edges / patches, composition pairs, real raws with one-slider sidecars via APFS clones);
`gen_tables.py` writes `crates/pipeline/src/tone_adobe.rs`. Keep outputs small: ≤1024 px synthetics,
Camera Raw's minimum size for real raws, extract to arrays and delete the TIFFs. Never touch a
Photoshop with open documents.
