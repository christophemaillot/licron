# LICRON Master Plan

> *This document describes the "big picture" goals and the major delivery phases for LICRON. These phases are intentionally product-oriented (what users can do) rather than implementation-oriented.*

## 1) Problem Statement
Teams often outgrow plain cron but do not want to adopt a heavy orchestrator. They need a simple way to schedule scripts from a directory with clear runtime behavior, safe defaults, and good operational visibility.

A key pain point with traditional cron is split ownership:
- script logic lives in one place,
- schedule lives somewhere else (`crontab`, `/etc/cron.d`, etc.).

This separation increases drift risk, makes reviews harder, and complicates backup/restore.

## 2) Vision
LICRON is the simplest reliable way to run scheduled file-based jobs on Linux:
- users drop executable files in a jobs directory,
- define schedule metadata in the same file header as the script logic,
- get predictable execution, clear logs, and safe process handling,
- operate it as a small daemon with minimal infrastructure.

Success means teams can run and evolve scheduled jobs confidently without custom glue code or complex platforms.
In practice, job code + schedule move together in one change, one review, one backup unit.

## 3) Phases (Product-Oriented)

### Phase 1 — Core Usage
Users can:
- run executable files from one jobs directory,
- keep script + schedule together in one file,
- get overlap protection, timeout limits, and clear skip/ignore logs.

### Phase 2 — Reliable Daemon Operations
Users can:
- add/update/remove jobs without daemon restart,
- rely on safe reload behavior and single-instance protection,
- stop LICRON gracefully without orphaning child processes.

### Phase 3 — Production Adoption
Users can:
- install and operate LICRON on Linux with systemd,
- troubleshoot quickly with structured logs and examples,
- trust behavior through integration-tested critical paths.

## 4) Non-Goals (for v1)
- Distributed/cluster scheduling.
- Persistent state database and missed-run replay.
- Multi-directory orchestration as a first-class feature.
- Full workflow orchestration (DAGs, dependencies, retries as a platform).
- UI/API platform scope.

---

This Master Plan is a compass, not a contract. Phases may be reordered, split, merged, or skipped as real usage feedback arrives.
