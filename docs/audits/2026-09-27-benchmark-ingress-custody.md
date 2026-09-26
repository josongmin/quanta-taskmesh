# Benchmark ingress and metadata custody

Base: protected main `930c150b35a3320095d3d38af5da63c5f4cfa407` (PR #5).
Concurrent benchmark-owner commits `6f16118`, `494b159`, and `5be7e09` were
compared against this base before selective integration. This change retains
PR #5's acquisition owner registry, bounded capture, regular-file stdin,
terminal execution flags, and complete control-arm denominator.

## Independently reproduced gaps

- A present FIFO Cargo config produced the same effective context as absent
  config. Discovery now treats present nonregular/dangling config as an error.
- Allocation input and qualification file identity could wait on a FIFO open.
  Shared descriptor ingress opens nonblocking and rejects nonregular descriptors.
  Regular symlinks remain subject to each caller's existing confinement rules.
- A zero-exit allocation producer could leave a live child. The producer now
  uses the existing acquisition owner and rejects every abnormal terminal flag;
  an unset return code maps to failure rather than `SystemExit(None)` success.
  The shell entrypoint uses managed `uv` Python and inventory declares its
  `psutil` dependency; standalone PATH fixtures exercise this same entrypoint.
- Metadata identity/archive commands were not owned or bounded. A binary
  inspection API preserves invalid UTF-8, NUL paths, and archive bytes, with
  30-second deadline, 0.5-second grace, and 64 MiB combined capture limit.
  Incomplete execution or overflow rejects identity rather than using partial
  output. Optional metadata commands propagate incomplete execution.
- Cross-rate workload-shape and class-mix changes could be admitted together as
  a capacity grid. Admission now binds the common workload and public full-host
  IO/blocking/CPU scope across every rate. Other modes require separate claims.
- Special validation hashed retained paths and subsequently launched their
  mutable originals. Validation now executes private copies of the exact bytes
  admitted by hashing. Calibration is parsed and hashed from one retained read.

## Verification and limits

Tests exercise real FIFO/config ingress, actual metadata hangs and live groups,
invalid UTF-8 and archive byte identity, producer abnormal flags, cross-rate
mutations, and validator/raw pathname replacement after digest checks. Full CI
qualification requires a clean final commit, all 16 local gates, the PR synthetic
merge source, and protected main. Changing-source owner tests are diagnostic.

The shared helpers do not bound stalled regular-file kernel/network I/O or
provide a filesystem sandbox against arbitrary hostile concurrent writers.
Metadata inspection uses ordinary owned process-group cleanup; cooperative
acquisition descendants retain PR #5's stronger nested ownership contract.
Execution limits are not performance SLOs. Longitudinal recovery/soak, additional
repeated measurement modes, B00 values, consumer profiles, matched peers, and
nightly/release qualification remain separate requirements; no thresholds or
measurement results are invented here.
