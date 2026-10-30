# Natural Earth land

`crates/commander/data/land-110m.txt` (header `# hamspot land v1`, ~64 KB) is
derived from Natural Earth 1:110m Land (`geojson/ne_110m_land.geojson` in
https://github.com/nvkelso/natural-earth-vector, commit 693f114, 2020-12-13):
polygon rings (P outer, H hole) as lon,lat pairs at two decimals. It is
compiled into the binary. It replaced the 1:110m coastline file; the coast
outline is now drawn from these rings.

Licence: public domain. "All versions of Natural Earth raster + vector map
data ... are in the public domain." https://www.naturalearthdata.com/about/terms-of-use/
Made with Natural Earth.
