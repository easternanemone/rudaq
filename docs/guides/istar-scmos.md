# iStar-sCMOS Hardware Guide

## Overview

Intensified sCMOS camera for time-resolved imaging.

## Key Specs

| Parameter | Value |
|-----------|-------|
| Sensor | 2560 x 2160 pixels |
| MCP Gain | 0-4095 |
| Gating | Down to <2ns |

## MCP Gain Control

```c
AT_SetInt(handle, L"MCPGain", 2000);  // 0-4095
```

**Warning:** High gain + bright light can damage phosphor.

## DDG Timing (10ps resolution)

```c
AT_SetFloat(handle, L"GateWidth", 100.0);   // 100ns gate
AT_SetFloat(handle, L"GateDelay", 50.0);    // 50ns delay
```

## Phosphor Selection

| Type | Decay | Use Case |
|------|-------|----------|
| P43 | ~1ms | Standard |
| P46 | ~300ns | High frame rates |

## See Also

- [Andor SDK3 Guide](andor-sdk3.md)
- [iStar PDF](../reference/iStar_sCMOS_Hardware_Guide%20(1).pdf)
