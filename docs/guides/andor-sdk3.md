# Andor SDK3 Driver Development Guide

## Overview

SDK3 is Andor's modern camera control API for sCMOS cameras including iStar-sCMOS.

## Feature Types

| Type | Get | Set |
|------|-----|-----|
| Integer | AT_GetInt | AT_SetInt |
| Float | AT_GetFloat | AT_SetFloat |
| Boolean | AT_GetBool | AT_SetBool |
| Enum | AT_GetEnumIndex | AT_SetEnumString |
| Command | N/A | AT_Command |

## Buffer Management

1. Allocate buffers (8-byte aligned)
2. Queue with AT_QueueBuffer
3. Start with AT_Command(handle, L"AcquisitionStart")
4. Wait with AT_WaitBuffer
5. Stop with AT_Command(handle, L"AcquisitionStop")

## See Also

- [iStar-sCMOS Guide](istar-scmos.md)
- [Andor SDK3 PDF](../reference/Andor%20Software%20Development%20Kit%203%20(1).pdf)
