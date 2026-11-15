# Benchmark fixtures

Both files are re-encodes of one Creative Commons Zero photograph. Metadata was stripped with `exiftool -all=`.

## large_photo.jpg

- Commons page: https://commons.wikimedia.org/wiki/File:Vienna_(14393257265).jpg
- Author: fotokoci
- License: CC0
- Original resolution: 3968×2232
- Modifications: resized to 3200×1800, re-encoded as JPEG quality 80, all metadata removed
- sha256: `6729655d3563176a62e456acb1e786645f5777d76298be8d9b7bd0bbde99307f`

## medium_photo.jpg

- Commons page: https://commons.wikimedia.org/wiki/File:Vienna_(14393257265).jpg
- Author: fotokoci
- License: CC0
- Original resolution: 3968×2232
- Modifications: resized to 800×450, re-encoded as JPEG quality 60, all metadata removed
- sha256: `dbf1e901f162b807e09ae7e2af1568dd1c870bde833a253d0bda3e39402f1070`

License confirmed from the Wikimedia Commons API (`extmetadata.LicenseShortName` = `CC0`) before the files were written.
