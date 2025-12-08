// Type definitions for experiment sequencer

export type ActionType =
  | 'move_absolute'
  | 'move_relative'
  | 'set_parameter'
  | 'trigger'
  | 'read'
  | 'loop'
  | 'delay'
  | 'parallel';

export interface ActionBlock {
  id: string;
  type: ActionType;
  params: Record<string, any>;
  children?: ActionBlock[];  // For loops/parallel blocks
}

export interface ActionTemplate {
  type: ActionType;
  label: string;
  description: string;
  icon: string;
  defaultParams: Record<string, any>;
  paramSchema: ParamDefinition[];
  canHaveChildren: boolean;
}

export interface ParamDefinition {
  name: string;
  label: string;
  type: 'string' | 'number' | 'boolean' | 'device' | 'select';
  options?: string[];
  default?: any;
  min?: number;
  max?: number;
  unit?: string;
  required?: boolean;
}

export interface ExperimentPlan {
  name: string;
  description: string;
  created: string;
  modified: string;
  actions: ActionBlock[];
}

// Action templates available in palette
export const ACTION_TEMPLATES: ActionTemplate[] = [
  {
    type: 'move_absolute',
    label: 'Move Absolute',
    description: 'Move stage to absolute position',
    icon: '→',
    defaultParams: {
      device: '',
      position: 0.0,
      wait: true,
    },
    paramSchema: [
      {
        name: 'device',
        label: 'Device',
        type: 'device',
        required: true,
      },
      {
        name: 'position',
        label: 'Position',
        type: 'number',
        default: 0.0,
        unit: 'mm',
        required: true,
      },
      {
        name: 'wait',
        label: 'Wait for completion',
        type: 'boolean',
        default: true,
      },
    ],
    canHaveChildren: false,
  },
  {
    type: 'move_relative',
    label: 'Move Relative',
    description: 'Move stage by relative amount',
    icon: '↔',
    defaultParams: {
      device: '',
      distance: 0.0,
      wait: true,
    },
    paramSchema: [
      {
        name: 'device',
        label: 'Device',
        type: 'device',
        required: true,
      },
      {
        name: 'distance',
        label: 'Distance',
        type: 'number',
        default: 0.0,
        unit: 'mm',
        required: true,
      },
      {
        name: 'wait',
        label: 'Wait for completion',
        type: 'boolean',
        default: true,
      },
    ],
    canHaveChildren: false,
  },
  {
    type: 'set_parameter',
    label: 'Set Parameter',
    description: 'Set device parameter (exposure, wavelength, etc.)',
    icon: '⚙',
    defaultParams: {
      device: '',
      parameter: '',
      value: 0,
    },
    paramSchema: [
      {
        name: 'device',
        label: 'Device',
        type: 'device',
        required: true,
      },
      {
        name: 'parameter',
        label: 'Parameter',
        type: 'select',
        options: ['exposure', 'wavelength', 'power'],
        required: true,
      },
      {
        name: 'value',
        label: 'Value',
        type: 'number',
        required: true,
      },
    ],
    canHaveChildren: false,
  },
  {
    type: 'trigger',
    label: 'Trigger',
    description: 'Trigger camera or acquisition',
    icon: '📷',
    defaultParams: {
      device: '',
      count: 1,
    },
    paramSchema: [
      {
        name: 'device',
        label: 'Device',
        type: 'device',
        required: true,
      },
      {
        name: 'count',
        label: 'Frame count',
        type: 'number',
        default: 1,
        min: 1,
      },
    ],
    canHaveChildren: false,
  },
  {
    type: 'read',
    label: 'Read',
    description: 'Read scalar value from device',
    icon: '📊',
    defaultParams: {
      device: '',
      variable: 'value',
    },
    paramSchema: [
      {
        name: 'device',
        label: 'Device',
        type: 'device',
        required: true,
      },
      {
        name: 'variable',
        label: 'Store in variable',
        type: 'string',
        default: 'value',
      },
    ],
    canHaveChildren: false,
  },
  {
    type: 'loop',
    label: 'Loop',
    description: 'Repeat actions N times',
    icon: '🔄',
    defaultParams: {
      iterations: 10,
      variable: 'i',
    },
    paramSchema: [
      {
        name: 'iterations',
        label: 'Iterations',
        type: 'number',
        default: 10,
        min: 1,
        required: true,
      },
      {
        name: 'variable',
        label: 'Loop variable',
        type: 'string',
        default: 'i',
      },
    ],
    canHaveChildren: true,
  },
  {
    type: 'delay',
    label: 'Delay',
    description: 'Wait for specified duration',
    icon: '⏱',
    defaultParams: {
      duration: 1.0,
    },
    paramSchema: [
      {
        name: 'duration',
        label: 'Duration',
        type: 'number',
        default: 1.0,
        min: 0,
        unit: 'seconds',
        required: true,
      },
    ],
    canHaveChildren: false,
  },
  {
    type: 'parallel',
    label: 'Parallel',
    description: 'Execute actions in parallel',
    icon: '⫴',
    defaultParams: {},
    paramSchema: [],
    canHaveChildren: true,
  },
];
