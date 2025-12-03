// Validation types for experiment plans

export type ValidationSeverity = 'error' | 'warning' | 'info';

export interface ValidationIssue {
  severity: ValidationSeverity;
  message: string;
  actionId?: string;  // ID of the problematic action block
  field?: string;     // Specific parameter field with issue
  suggestion?: string; // Suggested fix
}

export interface ValidationResult {
  valid: boolean;
  errors: ValidationIssue[];
  warnings: ValidationIssue[];
  infos: ValidationIssue[];
  estimatedDuration: number; // seconds
}

export interface DeviceMetadata {
  id: string;
  name: string;
  capabilities: {
    movable?: {
      min_position: number;
      max_position: number;
      position_units: string;
      max_speed?: number; // units per second
      default_speed?: number;
    };
    readable?: {
      reading_units: string;
      min_value?: number;
      max_value?: number;
    };
    frameProducer?: {
      min_exposure_ms: number;
      max_exposure_ms: number;
      readout_time_ms: number;
      max_frame_rate: number;
    };
    exposureControl?: {
      min_exposure_ms: number;
      max_exposure_ms: number;
    };
    wavelengthTunable?: {
      min_wavelength_nm: number;
      max_wavelength_nm: number;
      tuning_time_ms: number;
    };
    shutter?: {
      open_time_ms: number;
      close_time_ms: number;
    };
  };
}

export interface DryRunStep {
  actionId: string;
  actionType: string;
  description: string;
  estimatedTime: number; // seconds
  simulatedResult?: any;
}

export interface DryRunResult {
  success: boolean;
  steps: DryRunStep[];
  totalDuration: number;
  errors: string[];
}
