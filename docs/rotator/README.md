# Rotator reference manuals

The PDFs are third-party Yaesu documents. They are not committed; `docs/rotator/manuals/` is
in `.gitignore`. Download them again with the commands below.

| File (in `docs/rotator/manuals/`) | Model(s) | Text layer | Source |
| --- | --- | --- | --- |
| `G-800DXA_1000DXA_2800DXA_OM_2102B.pdf` | G-800DXA, G-1000DXA, G-2800DXA user manual (2102B-BS, current) | yes | https://www.yaesu.com/Files/4CB6273C-1018-01AF-FA4D504B591F641A/G-800DXA_1000DXA_2800DXA_OM_ENG_EAA67X101_2102B-BS.pdf |
| `G-1000DXA_Manual_limmared.pdf` | G-1000DXA user manual (older print, scanned) | no | https://manual.limmared.nu/Yaesu/G-1000DXA/ |
| `G-1000DXA_Service_Manual.pdf` | G-1000DXA service manual | yes | https://manual.limmared.nu/Yaesu/G-1000DXA/ |
| `G-2800DXA_Manual_limmared.pdf` | G-2800DXA user manual (scanned) | no | https://manual.limmared.nu/Yaesu/G-2800DXA/ |
| `G-800S_SDX_G-1000S_SDX_radiomanual.pdf` | G-800S/SDX, G-1000S/SDX instruction manual (scanned) | no | http://www.radiomanual.info/schemi/YAESU_ACC/Yaesu_G-800S_SDX_G-1000S_SDX_user.pdf |

```sh
mkdir -p docs/rotator/manuals && cd docs/rotator/manuals
curl -L -o G-800DXA_1000DXA_2800DXA_OM_2102B.pdf 'https://www.yaesu.com/Files/4CB6273C-1018-01AF-FA4D504B591F641A/G-800DXA_1000DXA_2800DXA_OM_ENG_EAA67X101_2102B-BS.pdf'
curl -L -o G-1000DXA_Manual_limmared.pdf 'https://manual.limmared.nu/Yaesu/G-1000DXA/G-1000DXA%20-%20Manual.pdf'
curl -L -o G-1000DXA_Service_Manual.pdf 'https://manual.limmared.nu/Yaesu/G-1000DXA/G-1000DXA%20-%20Service%20Manual.pdf'
curl -L -o G-2800DXA_Manual_limmared.pdf 'https://manual.limmared.nu/Yaesu/G-2800DXA/G-2800DXA%20-%20Manual.pdf'
curl -L -o G-800S_SDX_G-1000S_SDX_radiomanual.pdf 'http://www.radiomanual.info/schemi/YAESU_ACC/Yaesu_G-800S_SDX_G-1000S_SDX_user.pdf'
```

See `G-1000-notes.md` for what the manuals say about overlap, speed and preset.
