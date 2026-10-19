# Confirmation eval on the merged commit (2026-10-19)

This run uses the same 25 tasks, model, prices, sandbox and eval policy as the earlier baselines. The binary was built at `0e4b34c`, the commit merged as #94 and released as 0.8.0. Compared with `../2026-10-19-deepseek-flash-after` (`31e39f9`), it adds the final review fixes and the simplified shell policy.

| | before a / b | after a / b | final a / b |
|---|---|---|---|
| M1 cost per solved task | $0.0338 / $0.0305 | $0.0060 / $0.0055 | **$0.0060 / $0.0070** |
| M2 median rounds per task | 50 / 45 | 12 / 12 | **15 / 14** |
| M3 pass rate | 23/25 / 24/25 | 25/25 / 25/25 | **25/25 / 25/25** |
| total cost | $0.777 / $0.733 | $0.151 / $0.139 | $0.151 / $0.175 |

- **The pass rate holds at 50/50.**
- **Median rounds rose from 12 to 14–15.** A likely cause is the stricter shell rule: it refuses more read-only commands in the eval (globs, chains, pipes into anything but `head`, `tail`, `wc`, `grep`, `cut`, `tr` and `nl`). That costs a retry round.
- **Against the before baseline:**
  - Cost per solved task fell by about 79%: $0.032 to $0.0065 (mean of both runs).
  - Median rounds fell by about 69%: 47.5 to 14.5.
