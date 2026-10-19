# Opt-in machine reports use separate Git branches

Machine reporting will use a separate branch per installation in the configured
dotfiles remote. Reports contain revision, timestamp, outcome and counts, not
configuration values or raw logs. Cached observations remain available offline
and are labeled with their age; they are not a live machine connection.

The user accepted this approach in the 2026-10-19 design interview to support
machine visibility without a separate hosted service. Separate report refs and
commit construction preserve the configuration branch and actual staging area.
This still creates Git objects and branches; branches are not a privacy boundary
and may be visible to repository integrations. Stable installation identity must
prevent collisions between machines with the same display name.

Reporting is opt-in and authorizes automatic report commits and pushes only. It
is an explicit exception to ADR 0005's manual configuration publication workflow.
Configuration publication stays manual. Local application, report upload and
report retrieval have independent results so a transport failure cannot be
misrepresented as an apply failure or a successful report. Detailed activity
history remains local by default. Branch lifecycle, payload schema and retention
mechanics require implementation detail in the spec.
