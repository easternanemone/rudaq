import { ActionBlock, ExperimentPlan } from '../types/experiment';
import { DeviceInfo } from '../App';
import {
  ValidationResult,
  ValidationIssue,
  DeviceMetadata,
} from '../types/validation';

/**
 * Convert DeviceInfo from gRPC to DeviceMetadata with estimated capabilities
 */
function createDeviceMetadata(device: DeviceInfo): DeviceMetadata {
  return {
    id: device.id,
    name: device.name,
    capabilities: {
      movable: device.is_movable
        ? {
            min_position: device.min_position ?? -100,
            max_position: device.max_position ?? 100,
            position_units: device.position_units ?? 'mm',
            max_speed: 10, // Default estimate
            default_speed: 5, // Default estimate
          }
        : undefined,
      readable: device.is_readable
        ? {
            reading_units: device.reading_units ?? 'units',
          }
        : undefined,
      frameProducer: device.is_frame_producer
        ? {
            min_exposure_ms: 0.1,
            max_exposure_ms: 60000,
            readout_time_ms: 50, // Typical camera readout
            max_frame_rate: 100,
          }
        : undefined,
      exposureControl: device.is_exposure_controllable
        ? {
            min_exposure_ms: 0.1,
            max_exposure_ms: 60000,
          }
        : undefined,
      wavelengthTunable: device.is_wavelength_tunable
        ? {
            min_wavelength_nm: 690,
            max_wavelength_nm: 1040,
            tuning_time_ms: 2000, // MaiTai typical tuning time
          }
        : undefined,
      shutter: device.is_shutter_controllable
        ? {
            open_time_ms: 100,
            close_time_ms: 100,
          }
        : undefined,
    },
  };
}

/**
 * Find a device by ID in the devices array
 */
function findDevice(
  devices: DeviceInfo[],
  deviceId: string
): DeviceMetadata | null {
  const device = devices.find((d) => d.id === deviceId);
  return device ? createDeviceMetadata(device) : null;
}

/**
 * Calculate the nesting depth of a block
 */
function calculateNestingDepth(
  block: ActionBlock,
  allBlocks: ActionBlock[],
  depth: number = 0
): number {
  let maxDepth = depth;

  if (block.children) {
    for (const child of block.children) {
      const childDepth = calculateNestingDepth(child, allBlocks, depth + 1);
      maxDepth = Math.max(maxDepth, childDepth);
    }
  }

  return maxDepth;
}

/**
 * Validate a single action block
 */
function validateAction(
  action: ActionBlock,
  devices: DeviceInfo[],
  issues: ValidationIssue[]
): void {
  switch (action.type) {
    case 'move_absolute':
    case 'move_relative':
      validateMoveAction(action, devices, issues);
      break;
    case 'set_parameter':
      validateSetParameterAction(action, devices, issues);
      break;
    case 'trigger':
      validateTriggerAction(action, devices, issues);
      break;
    case 'read':
      validateReadAction(action, devices, issues);
      break;
    case 'loop':
      validateLoopAction(action, issues);
      break;
    case 'delay':
      validateDelayAction(action, issues);
      break;
    case 'parallel':
      // Parallel blocks are structurally valid, just validate children
      break;
  }

  // Recursively validate children
  if (action.children) {
    for (const child of action.children) {
      validateAction(child, devices, issues);
    }
  }
}

function validateMoveAction(
  action: ActionBlock,
  devices: DeviceInfo[],
  issues: ValidationIssue[]
): void {
  const deviceId = action.params.device;

  if (!deviceId) {
    issues.push({
      severity: 'error',
      message: 'Move action missing device parameter',
      actionId: action.id,
      field: 'device',
      suggestion: 'Select a movable device',
    });
    return;
  }

  const device = findDevice(devices, deviceId);

  if (!device) {
    issues.push({
      severity: 'error',
      message: `Device '${deviceId}' not found`,
      actionId: action.id,
      field: 'device',
      suggestion: 'Check that the device is connected and registered',
    });
    return;
  }

  if (!device.capabilities.movable) {
    issues.push({
      severity: 'error',
      message: `Device '${device.name}' is not movable`,
      actionId: action.id,
      field: 'device',
      suggestion: 'Select a device with Movable capability',
    });
    return;
  }

  const movable = device.capabilities.movable;

  if (action.type === 'move_absolute') {
    const position = action.params.position;

    if (position === undefined || position === null) {
      issues.push({
        severity: 'error',
        message: 'Move absolute action missing position parameter',
        actionId: action.id,
        field: 'position',
      });
      return;
    }

    if (typeof position !== 'number') {
      issues.push({
        severity: 'error',
        message: 'Position must be a number',
        actionId: action.id,
        field: 'position',
      });
      return;
    }

    if (position < movable.min_position) {
      issues.push({
        severity: 'error',
        message: `Position ${position} ${movable.position_units} is below minimum ${movable.min_position} ${movable.position_units}`,
        actionId: action.id,
        field: 'position',
        suggestion: `Set position >= ${movable.min_position}`,
      });
    }

    if (position > movable.max_position) {
      issues.push({
        severity: 'error',
        message: `Position ${position} ${movable.position_units} exceeds maximum ${movable.max_position} ${movable.position_units}`,
        actionId: action.id,
        field: 'position',
        suggestion: `Set position <= ${movable.max_position}`,
      });
    }
  }

  if (action.type === 'move_relative') {
    const distance = action.params.distance;

    if (distance === undefined || distance === null) {
      issues.push({
        severity: 'error',
        message: 'Move relative action missing distance parameter',
        actionId: action.id,
        field: 'distance',
      });
      return;
    }

    if (typeof distance !== 'number') {
      issues.push({
        severity: 'error',
        message: 'Distance must be a number',
        actionId: action.id,
        field: 'distance',
      });
    }
  }
}

function validateSetParameterAction(
  action: ActionBlock,
  devices: DeviceInfo[],
  issues: ValidationIssue[]
): void {
  const deviceId = action.params.device;

  if (!deviceId) {
    issues.push({
      severity: 'error',
      message: 'Set parameter action missing device',
      actionId: action.id,
      field: 'device',
    });
    return;
  }

  const device = findDevice(devices, deviceId);

  if (!device) {
    issues.push({
      severity: 'error',
      message: `Device '${deviceId}' not found`,
      actionId: action.id,
      field: 'device',
    });
    return;
  }

  const parameter = action.params.parameter;
  const value = action.params.value;

  if (!parameter) {
    issues.push({
      severity: 'error',
      message: 'Set parameter action missing parameter name',
      actionId: action.id,
      field: 'parameter',
    });
    return;
  }

  if (value === undefined || value === null) {
    issues.push({
      severity: 'error',
      message: 'Set parameter action missing value',
      actionId: action.id,
      field: 'value',
    });
    return;
  }

  // Validate parameter-specific constraints
  if (parameter === 'exposure') {
    if (!device.capabilities.exposureControl) {
      issues.push({
        severity: 'error',
        message: `Device '${device.name}' does not support exposure control`,
        actionId: action.id,
        field: 'parameter',
      });
      return;
    }

    const exposure = device.capabilities.exposureControl;
    if (value < exposure.min_exposure_ms) {
      issues.push({
        severity: 'error',
        message: `Exposure ${value} ms is below minimum ${exposure.min_exposure_ms} ms`,
        actionId: action.id,
        field: 'value',
        suggestion: `Set exposure >= ${exposure.min_exposure_ms} ms`,
      });
    }

    if (value > exposure.max_exposure_ms) {
      issues.push({
        severity: 'error',
        message: `Exposure ${value} ms exceeds maximum ${exposure.max_exposure_ms} ms`,
        actionId: action.id,
        field: 'value',
        suggestion: `Set exposure <= ${exposure.max_exposure_ms} ms`,
      });
    }
  }

  if (parameter === 'wavelength') {
    if (!device.capabilities.wavelengthTunable) {
      issues.push({
        severity: 'error',
        message: `Device '${device.name}' does not support wavelength tuning`,
        actionId: action.id,
        field: 'parameter',
      });
      return;
    }

    const tunable = device.capabilities.wavelengthTunable;
    if (value < tunable.min_wavelength_nm) {
      issues.push({
        severity: 'error',
        message: `Wavelength ${value} nm is below minimum ${tunable.min_wavelength_nm} nm`,
        actionId: action.id,
        field: 'value',
        suggestion: `Set wavelength >= ${tunable.min_wavelength_nm} nm`,
      });
    }

    if (value > tunable.max_wavelength_nm) {
      issues.push({
        severity: 'error',
        message: `Wavelength ${value} nm exceeds maximum ${tunable.max_wavelength_nm} nm`,
        actionId: action.id,
        field: 'value',
        suggestion: `Set wavelength <= ${tunable.max_wavelength_nm} nm`,
      });
    }
  }
}

function validateTriggerAction(
  action: ActionBlock,
  devices: DeviceInfo[],
  issues: ValidationIssue[]
): void {
  const deviceId = action.params.device;

  if (!deviceId) {
    issues.push({
      severity: 'error',
      message: 'Trigger action missing device',
      actionId: action.id,
      field: 'device',
    });
    return;
  }

  const device = findDevice(devices, deviceId);

  if (!device) {
    issues.push({
      severity: 'error',
      message: `Device '${deviceId}' not found`,
      actionId: action.id,
      field: 'device',
    });
    return;
  }

  if (!device.capabilities.frameProducer) {
    issues.push({
      severity: 'error',
      message: `Device '${device.name}' is not a frame producer (camera)`,
      actionId: action.id,
      field: 'device',
      suggestion: 'Select a camera device',
    });
    return;
  }

  const count = action.params.count;
  if (count !== undefined && count < 1) {
    issues.push({
      severity: 'error',
      message: 'Frame count must be at least 1',
      actionId: action.id,
      field: 'count',
    });
  }
}

function validateReadAction(
  action: ActionBlock,
  devices: DeviceInfo[],
  issues: ValidationIssue[]
): void {
  const deviceId = action.params.device;

  if (!deviceId) {
    issues.push({
      severity: 'error',
      message: 'Read action missing device',
      actionId: action.id,
      field: 'device',
    });
    return;
  }

  const device = findDevice(devices, deviceId);

  if (!device) {
    issues.push({
      severity: 'error',
      message: `Device '${deviceId}' not found`,
      actionId: action.id,
      field: 'device',
    });
    return;
  }

  if (!device.capabilities.readable) {
    issues.push({
      severity: 'error',
      message: `Device '${device.name}' is not readable`,
      actionId: action.id,
      field: 'device',
      suggestion: 'Select a device with Readable capability',
    });
  }
}

function validateLoopAction(
  action: ActionBlock,
  issues: ValidationIssue[]
): void {
  const iterations = action.params.iterations;

  if (iterations === undefined || iterations === null) {
    issues.push({
      severity: 'error',
      message: 'Loop action missing iterations parameter',
      actionId: action.id,
      field: 'iterations',
    });
    return;
  }

  if (typeof iterations !== 'number' || iterations < 1) {
    issues.push({
      severity: 'error',
      message: 'Loop iterations must be a positive number',
      actionId: action.id,
      field: 'iterations',
    });
  }

  if (iterations > 10000) {
    issues.push({
      severity: 'warning',
      message: `Loop with ${iterations} iterations may take a very long time`,
      actionId: action.id,
      field: 'iterations',
      suggestion: 'Consider reducing iteration count for testing',
    });
  }

  if (!action.children || action.children.length === 0) {
    issues.push({
      severity: 'warning',
      message: 'Loop has no actions inside it',
      actionId: action.id,
      suggestion: 'Add actions to loop body',
    });
  }
}

function validateDelayAction(
  action: ActionBlock,
  issues: ValidationIssue[]
): void {
  const duration = action.params.duration;

  if (duration === undefined || duration === null) {
    issues.push({
      severity: 'error',
      message: 'Delay action missing duration parameter',
      actionId: action.id,
      field: 'duration',
    });
    return;
  }

  if (typeof duration !== 'number' || duration < 0) {
    issues.push({
      severity: 'error',
      message: 'Delay duration must be a non-negative number',
      actionId: action.id,
      field: 'duration',
    });
  }

  if (duration > 3600) {
    issues.push({
      severity: 'warning',
      message: `Delay of ${duration} seconds (${(duration / 60).toFixed(1)} minutes) is very long`,
      actionId: action.id,
      field: 'duration',
      suggestion: 'Verify this delay is intentional',
    });
  }
}

/**
 * Estimate the duration of a single action
 */
function estimateActionDuration(
  action: ActionBlock,
  devices: DeviceInfo[],
  currentPosition: Map<string, number> = new Map()
): number {
  let duration = 0;

  switch (action.type) {
    case 'move_absolute': {
      const device = findDevice(devices, action.params.device);
      if (device?.capabilities.movable) {
        const targetPosition = action.params.position ?? 0;
        const currentPos = currentPosition.get(device.id) ?? 0;
        const distance = Math.abs(targetPosition - currentPos);
        const speed = device.capabilities.movable.default_speed ?? 5;
        duration = distance / speed; // seconds
        currentPosition.set(device.id, targetPosition);
      }
      break;
    }
    case 'move_relative': {
      const device = findDevice(devices, action.params.device);
      if (device?.capabilities.movable) {
        const distance = Math.abs(action.params.distance ?? 0);
        const speed = device.capabilities.movable.default_speed ?? 5;
        duration = distance / speed;
        const currentPos = currentPosition.get(device.id) ?? 0;
        currentPosition.set(device.id, currentPos + (action.params.distance ?? 0));
      }
      break;
    }
    case 'set_parameter': {
      const device = findDevice(devices, action.params.device);
      if (action.params.parameter === 'wavelength' && device?.capabilities.wavelengthTunable) {
        duration = device.capabilities.wavelengthTunable.tuning_time_ms / 1000;
      } else if (action.params.parameter === 'exposure') {
        duration = 0.1; // Minimal time to set exposure
      } else {
        duration = 0.05; // Generic parameter set time
      }
      break;
    }
    case 'trigger': {
      const device = findDevice(devices, action.params.device);
      if (device?.capabilities.frameProducer) {
        const count = action.params.count ?? 1;
        const readoutTime = device.capabilities.frameProducer.readout_time_ms / 1000;
        // Assume exposure was set separately; just account for readout
        duration = count * readoutTime;
      }
      break;
    }
    case 'read': {
      duration = 0.05; // Minimal read time
      break;
    }
    case 'delay': {
      duration = action.params.duration ?? 0;
      break;
    }
    case 'loop': {
      const iterations = action.params.iterations ?? 1;
      let loopBodyDuration = 0;
      if (action.children) {
        for (const child of action.children) {
          loopBodyDuration += estimateActionDuration(child, devices, currentPosition);
        }
      }
      duration = iterations * loopBodyDuration;
      break;
    }
    case 'parallel': {
      // For parallel, take the maximum duration of children
      let maxChildDuration = 0;
      if (action.children) {
        for (const child of action.children) {
          const childDuration = estimateActionDuration(child, devices, new Map(currentPosition));
          maxChildDuration = Math.max(maxChildDuration, childDuration);
        }
      }
      duration = maxChildDuration;
      break;
    }
  }

  return duration;
}

/**
 * Estimate total experiment duration
 */
function estimateExperimentDuration(
  plan: ExperimentPlan,
  devices: DeviceInfo[]
): number {
  let totalDuration = 0;
  const currentPosition = new Map<string, number>();

  for (const action of plan.actions) {
    totalDuration += estimateActionDuration(action, devices, currentPosition);
  }

  return totalDuration;
}

/**
 * Check for excessive loop nesting
 */
function checkLoopNesting(
  actions: ActionBlock[],
  issues: ValidationIssue[]
): void {
  const MAX_NESTING_DEPTH = 3;

  function checkDepth(block: ActionBlock, depth: number = 0): void {
    if (block.type === 'loop') {
      if (depth >= MAX_NESTING_DEPTH) {
        issues.push({
          severity: 'warning',
          message: `Loop nesting depth ${depth + 1} exceeds recommended maximum of ${MAX_NESTING_DEPTH}`,
          actionId: block.id,
          suggestion: 'Deep nesting can be hard to debug and understand',
        });
      }

      if (block.children) {
        for (const child of block.children) {
          checkDepth(child, depth + 1);
        }
      }
    } else if (block.children) {
      for (const child of block.children) {
        checkDepth(child, depth);
      }
    }
  }

  for (const action of actions) {
    checkDepth(action);
  }
}

/**
 * Main validation function
 */
export function validateExperiment(
  plan: ExperimentPlan,
  devices: DeviceInfo[]
): ValidationResult {
  const allIssues: ValidationIssue[] = [];

  // Validate each action
  for (const action of plan.actions) {
    validateAction(action, devices, allIssues);
  }

  // Check for loop nesting issues
  checkLoopNesting(plan.actions, allIssues);

  // Warn if no actions
  if (plan.actions.length === 0) {
    allIssues.push({
      severity: 'warning',
      message: 'Experiment plan has no actions',
      suggestion: 'Add actions from the palette',
    });
  }

  // Estimate duration
  const estimatedDuration = estimateExperimentDuration(plan, devices);

  // Separate issues by severity
  const errors = allIssues.filter((i) => i.severity === 'error');
  const warnings = allIssues.filter((i) => i.severity === 'warning');
  const infos = allIssues.filter((i) => i.severity === 'info');

  return {
    valid: errors.length === 0,
    errors,
    warnings,
    infos,
    estimatedDuration,
  };
}

/**
 * Format duration as human-readable string
 */
export function formatDuration(seconds: number): string {
  if (seconds < 60) {
    return `${seconds.toFixed(1)}s`;
  }

  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = Math.floor(seconds % 60);

  const parts: string[] = [];
  if (hours > 0) parts.push(`${hours}h`);
  if (minutes > 0) parts.push(`${minutes}m`);
  if (secs > 0 || parts.length === 0) parts.push(`${secs}s`);

  return parts.join(' ');
}
