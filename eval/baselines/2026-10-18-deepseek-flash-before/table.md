| task | run-a pass/rounds/cost | run-b pass/rounds/cost |
| brain-todo-priorities | P 29 $0.0163 | P 27 $0.0132 |
| doc-architecture-notes | F 81 $0.0491 | P 57 $0.0315 |
| doc-incident-writeup | P 79 $0.0498 | P 45 $0.0215 |
| hist-auto-mode-choice | P 123 $0.0646 | P 118 $0.0572 |
| hist-custom-provider-not-catalog | P 62 $0.0194 | P 45 $0.0163 |
| hist-hook-tooltip-wraps | P 86 $0.0450 | P 92 $0.0396 |
| hist-jev-settings | P 212 $0.1426 | P 170 $0.1194 |
| hist-mtplx-engine-label | P 96 $0.0455 | P 145 $0.0635 |
| hist-pretooluse-hook-round | P 50 $0.0251 | P 60 $0.0227 |
| hist-windows-folder-name | P 159 $0.0785 | P 160 $0.0972 |
| js-callbacks-to-async | P 79 $0.0286 | P 50 $0.0159 |
| js-daily-summary-counts | P 25 $0.0084 | P 31 $0.0078 |
| js-digest-local-day | P 22 $0.0054 | P 18 $0.0058 |
| js-lru-cache | P 29 $0.0098 | P 32 $0.0110 |
| js-money-in-cents | P 50 $0.0186 | P 58 $0.0199 |
| js-slugify-accents | P 37 $0.0223 | P 20 $0.0063 |
| js-two-small-features | P 30 $0.0107 | P 28 $0.0099 |
| py-csv-quoted-fields | P 20 $0.0080 | P 19 $0.0086 |
| py-extract-validation | F 85 $0.0509 | P 117 $0.0521 |
| py-inventory-weight-report | P 25 $0.0065 | P 33 $0.0069 |
| py-pager-last-page | P 16 $0.0055 | P 19 $0.0055 |
| py-plan-quota-off-by-one | P 20 $0.0063 | P 21 $0.0062 |
| py-rename-timeout-setting | P 37 $0.0151 | P 55 $0.0188 |
| py-retry-backoff | P 82 $0.0382 | F 124 $0.0705 |
| py-two-unrelated-bugs | P 23 $0.0066 | P 18 $0.0053 |

run-a M1=$0.0338 M2=50 M3=23/25 cost=$0.7767 in_tokens=28471127 cache_read=95.5% out=838719 subagents=82 main_rounds_med=19 wall_med=164s
   FAIL doc-architecture-notes completed | docs/ARCHITECTURE.md is missing / 
   FAIL py-extract-validation completed |  /     from validators import validate_user / E   ModuleNotFoundError: No module named 'validators' / =========================== short test summary info ============================ / ERROR test_validators.py / !!!!!!!!!!!!!!!!!!!! Interrupted: 1 error during collection !!!!!!!!!!!!!!!!!!!! / 1 error in 0.12s / 
run-b M1=$0.0305 M2=45 M3=24/25 cost=$0.7327 in_tokens=30575591 cache_read=95.9% out=759908 subagents=75 main_rounds_med=22 wall_med=89s
   FAIL py-retry-backoff completed | etry import retry / E   ModuleNotFoundError: No module named 'retry' / =========================== short test summary info ============================ / ERROR test_retry_basic.py / ERROR test_retry_hidden.py / !!!!!!!!!!!!!!!!!!! Interrupted: 2 errors during collection !!!!!!!!!!!!!!!!!!!! / 2 errors in 0.10s / 
