# SPIRIT Laser Guide

## Overview

High-energy ultrafast laser (1040nm, <400fs).

## Key Specs

| Parameter | Value |
|-----------|-------|
| Wavelength | 1040 nm |
| Pulse Duration | <400 fs |
| Rep Rate | Single shot - 1 MHz |

## TCP/IP Interface (Port 9000)

| Command | Description |
|---------|-------------|
| STATUS? | System status |
| SHUTTER OPEN | Open shutter |
| SHUTTER CLOSE | Close shutter |
| POWER? | Query power |

## CANopen Interface (Node 0x0E)

| Index | Description |
|-------|-------------|
| 0x2000 | State control |
| 0x2001 | Shutter control |
| 0x2002 | Power setpoint |

## State Machine

0=OFF, 1=STANDBY, 2=WARMING, 3=READY, 4=EMISSION, 11=FAULT

## See Also

- [SPIRIT PDF](../reference/Manual%20SPIRIT%201040-16_30-HE%20Rev12%20(1).pdf)
