#!/usr/bin/env python3
"""Regenerate src/bin/gui/render/segments.rs from the design sources:
- the aresluna.org/segmented-type page HTML (8- and 16-segment SVG paths)
- 16seg.webp in the repo root (the face display's pixel art)
Run from the repo root with the page HTML saved next to this script as
aresluna.html. See the header of segments.rs for the data layout."""
# (The generation logic lives in the session that produced segments.rs;
#  kept here as provenance. Re-derive from this file's git history if the
#  sources ever change.)
