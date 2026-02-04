/**
 * @file wrapper.hpp
 * @brief Bindgen wrapper for Dover Motion MotionSynergyAPI C++ headers
 *
 * This file includes all necessary Dover Motion SDK headers for FFI binding
 * generation. It follows the structure documented in:
 *
 * Dover Motion - Motion Synergy API User Manual
 * Section 6: C++ Software Integration (pp. 121-163)
 *
 * Key classes (per Section 6.2):
 * - imp::IAxisDevice (Section 6.2.2)
 * - imp::MotionSynergyAPI (Section 6.2.10)
 * - imp::CommunicationSettings (Section 6.2.1)
 * - imp::IMotionControllerConfiguration (Section 6.2.7)
 *
 * Critical functions for LIBS experiments:
 * - EnableTriggerOnPosition() / DisableTriggerOnPosition()
 * - MoveAbsolute() / MoveRelative()
 * - GetActualPosition() / GetCommandedPosition()
 * - SetVelocity() / SetAcceleration() / SetDeceleration()
 */

// Include main Dover Motion SDK headers
// Note: Actual header names may differ - adjust based on SDK installation
#ifdef DOVER_SDK_AVAILABLE
#include <MotionSynergyAPI.h>
#include <IAxisDevice.h>
#include <CommunicationSettings.h>
#include <IMotionControllerConfiguration.h>
#include <MotionErrorSettings.h>
#include <MotionTrackingSettings.h>
#else
// Placeholder when SDK is not available - build.rs will generate dummy bindings
#warning "Dover Motion SDK headers not found - using placeholder types"
#endif
