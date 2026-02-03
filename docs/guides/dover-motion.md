# Dover Motion Synergy API Guide

## Overview

Motion control API for high-precision stages.

## Core Functions

```c
Synergy_Initialize();
Synergy_Open("USB:0", &handle);
Synergy_MoveAbsolute(handle, axis, position);
Synergy_GetPosition(handle, axis, &pos);
```

## Motion Commands

| Function | Description |
|----------|-------------|
| MoveAbsolute | Move to position |
| MoveRelative | Move by distance |
| MoveVelocity | Continuous motion |
| Stop | Stop motion |
| Home | Execute homing |

## Trigger-on-Position (LIBS)

```c
Synergy_SetTriggerPositions(handle, axis, positions, count);
Synergy_EnableTrigger(handle, axis, true);
Synergy_MoveAbsolute(handle, axis, end_pos);
```

## Communication Interfaces

| Interface | Address |
|-----------|---------|
| USB | USB:0 |
| Ethernet | IP:192.168.1.100 |

## See Also

- [Dover Motion API Manual](../reference/Motion%20Synergy%20API%20User%20Manual%20v20230707.pdf)
