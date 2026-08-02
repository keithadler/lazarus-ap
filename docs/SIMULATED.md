# Simulated devices — what is real here and what is not

The emulator reproduces the AP-101S and its I/O processor from IBM's
own documentation. The *devices on the far end of the buses* are a
different matter, and this file draws that line explicitly.

## The rule

**Protocol: real. Data: invented.**

`src/subsystems.rs` answers the serial-bus protocol faithfully — the
command word format, interface-unit addressing, data-word framing and
validity bits are all per Appendix III, which is why genuine flight
code polls these devices successfully and why a receiver's own error
checks catch a corrupted reply. But every *value* returned is made up
by this project. None of it is telemetry. None of it came from a
Shuttle. It must never be presented as though it did.

## Why simulate instead of emulate

The flight routines that read an inertial measurement unit consume
three numbers. Reproducing the IMU's internals — its gyros, its
calibration, its built-in test — would take months and change nothing
those routines see. A device that answers with plausible values unlocks
the code path immediately.

The judgement call per device is: **how much of the device's behaviour
does the flight code actually inspect?**

- Pure data sources (attitude, air data, accelerations) — simulate the
  numbers; the code does arithmetic on them and nothing else.
- Devices with handshakes and status words (mass memory) — a thin
  protocol shell is needed, because the code checks completion before
  trusting data. That shell is small.
- Devices whose *timing* is the point — not relevant here; nothing is
  flying.

## What exists

| Device | Models | Data |
|---|---|---|
| `DataSource` | generic polled sensor: command in, burst of words out | invented |
| `DataSource::imu` | inertial unit reporting roll/pitch/yaw | invented |
| `DataSource::air_data` | altitude, airspeed, validity flag | invented |
| `MassMemory` | block select + read, the tape loads between flight phases | invented |

Fault injection is built in: set `garble` and the device answers with
corrupted validity bits, which the receiving BCE rejects — proving the
error checking works rather than asserting it does.

Tests: `tests/sensors.rs`.

## The closed loop

`tests/sensors.rs::flight_math_on_simulated_sensor_data` runs the whole
chain: a simulated sensor reports a direction over the serial bus, a
BCE program polls it into main storage, and then genuine NASA flight
routines - VV10S3 (UNIT VECTOR), which reaches through VV0SN into
SQRT, followed by VV6S3 (dot product) - turn it into a pointing angle.

    sensor reports (3, 3, 0)  ->  cos to the +X reference = 0.7071068
                                  i.e. 45 degrees off axis

The direction is invented. Every instruction that processed it is the
Shuttle's own. This is the shape of real guidance work: read a vector
from a sensor, normalise it, dot it with where you want to point.
