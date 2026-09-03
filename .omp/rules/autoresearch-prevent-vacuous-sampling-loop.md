---
name: autoresearch-prevent-vacuous-sampling-loop
description: "Stop vacuous repeated sampling loops when no substantive code hypothesis is being tested"
condition: "(?:Sample run to measure|Second run to|Third run to|Fourth run to|Fifth run to|Re-run clean baseline)[^\\n]*(?:variance|distribution|stability|mean latency)"
scope: "tool:run_experiment"
---

Do not repeatedly run benchmarks across consecutive turns merely to sample noise or re-confirm an unchanged baseline. If code has converged and all plausible hypotheses from the backlog are tested, stop looping, state convergence, report the final best metric and delta, and yield to the user instead of spinning.