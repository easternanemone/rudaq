import { useMemo } from 'react';
import { AlertCircle, AlertTriangle, Info, CheckCircle, Play, Zap, Clock } from 'lucide-react';
import { ExperimentPlan } from '../types/experiment';
import { DeviceInfo } from '../App';
import { ValidationResult, ValidationIssue } from '../types/validation';
import { validateExperiment, formatDuration } from '../utils/experimentValidator';

interface ExperimentValidationProps {
  plan: ExperimentPlan;
  devices: DeviceInfo[];
  onRunExperiment?: () => void;
  onDryRun?: () => void;
  onSelectIssue?: (actionId: string) => void;
}

function ValidationIssueItem({
  issue,
  onSelect
}: {
  issue: ValidationIssue;
  onSelect?: (actionId: string) => void;
}) {
  const getIcon = () => {
    switch (issue.severity) {
      case 'error':
        return <AlertCircle className="w-4 h-4 text-red-400" />;
      case 'warning':
        return <AlertTriangle className="w-4 h-4 text-yellow-400" />;
      case 'info':
        return <Info className="w-4 h-4 text-blue-400" />;
    }
  };

  const getColor = () => {
    switch (issue.severity) {
      case 'error':
        return 'text-red-300 bg-red-900/20 border-red-800';
      case 'warning':
        return 'text-yellow-300 bg-yellow-900/20 border-yellow-800';
      case 'info':
        return 'text-blue-300 bg-blue-900/20 border-blue-800';
    }
  };

  const handleClick = () => {
    if (issue.actionId && onSelect) {
      onSelect(issue.actionId);
    }
  };

  return (
    <div
      className={`p-3 rounded border ${getColor()} ${
        issue.actionId ? 'cursor-pointer hover:opacity-80' : ''
      } transition-opacity`}
      onClick={handleClick}
    >
      <div className="flex items-start gap-2">
        {getIcon()}
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium">{issue.message}</p>
          {issue.field && (
            <p className="text-xs opacity-75 mt-0.5">Field: {issue.field}</p>
          )}
          {issue.suggestion && (
            <p className="text-xs mt-1 opacity-90">
              💡 {issue.suggestion}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

function ValidationSummary({ result }: { result: ValidationResult }) {
  const { valid, errors, warnings, estimatedDuration } = result;

  return (
    <div className="grid grid-cols-3 gap-4 mb-4">
      {/* Status */}
      <div
        className={`p-4 rounded border ${
          valid
            ? 'bg-green-900/20 border-green-800 text-green-300'
            : 'bg-red-900/20 border-red-800 text-red-300'
        }`}
      >
        <div className="flex items-center gap-2 mb-1">
          {valid ? (
            <CheckCircle className="w-5 h-5" />
          ) : (
            <AlertCircle className="w-5 h-5" />
          )}
          <span className="font-semibold">
            {valid ? 'Valid' : 'Invalid'}
          </span>
        </div>
        <p className="text-xs opacity-75">
          {errors.length} error{errors.length !== 1 ? 's' : ''}, {warnings.length} warning{warnings.length !== 1 ? 's' : ''}
        </p>
      </div>

      {/* Duration */}
      <div className="p-4 rounded border bg-blue-900/20 border-blue-800 text-blue-300">
        <div className="flex items-center gap-2 mb-1">
          <Clock className="w-5 h-5" />
          <span className="font-semibold">Duration</span>
        </div>
        <p className="text-lg font-mono">{formatDuration(estimatedDuration)}</p>
      </div>

      {/* Actions */}
      <div className="p-4 rounded border bg-slate-800 border-slate-700 text-slate-300">
        <div className="flex items-center gap-2 mb-1">
          <Zap className="w-5 h-5" />
          <span className="font-semibold">Ready</span>
        </div>
        <p className="text-xs opacity-75">
          {valid ? 'Experiment can be executed' : 'Fix errors to proceed'}
        </p>
      </div>
    </div>
  );
}

export default function ExperimentValidation({
  plan,
  devices,
  onRunExperiment,
  onDryRun,
  onSelectIssue,
}: ExperimentValidationProps) {
  const validationResult = useMemo(() => {
    return validateExperiment(plan, devices);
  }, [plan, devices]);

  const { valid, errors, warnings, infos } = validationResult;

  return (
    <div className="h-full flex flex-col bg-slate-900">
      {/* Header */}
      <div className="px-4 py-3 bg-slate-800 border-b border-slate-700">
        <h2 className="text-lg font-bold">Experiment Validation</h2>
        <p className="text-xs text-slate-400 mt-0.5">
          Review validation results before running
        </p>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        <ValidationSummary result={validationResult} />

        {/* Errors */}
        {errors.length > 0 && (
          <div>
            <h3 className="text-sm font-semibold text-red-400 mb-2 flex items-center gap-2">
              <AlertCircle className="w-4 h-4" />
              Errors ({errors.length})
            </h3>
            <div className="space-y-2">
              {errors.map((error, i) => (
                <ValidationIssueItem
                  key={i}
                  issue={error}
                  onSelect={onSelectIssue}
                />
              ))}
            </div>
          </div>
        )}

        {/* Warnings */}
        {warnings.length > 0 && (
          <div>
            <h3 className="text-sm font-semibold text-yellow-400 mb-2 flex items-center gap-2">
              <AlertTriangle className="w-4 h-4" />
              Warnings ({warnings.length})
            </h3>
            <div className="space-y-2">
              {warnings.map((warning, i) => (
                <ValidationIssueItem
                  key={i}
                  issue={warning}
                  onSelect={onSelectIssue}
                />
              ))}
            </div>
          </div>
        )}

        {/* Infos */}
        {infos.length > 0 && (
          <div>
            <h3 className="text-sm font-semibold text-blue-400 mb-2 flex items-center gap-2">
              <Info className="w-4 h-4" />
              Information ({infos.length})
            </h3>
            <div className="space-y-2">
              {infos.map((info, i) => (
                <ValidationIssueItem
                  key={i}
                  issue={info}
                  onSelect={onSelectIssue}
                />
              ))}
            </div>
          </div>
        )}

        {/* All clear message */}
        {errors.length === 0 && warnings.length === 0 && infos.length === 0 && plan.actions.length > 0 && (
          <div className="p-6 text-center">
            <CheckCircle className="w-12 h-12 text-green-400 mx-auto mb-3" />
            <h3 className="text-lg font-semibold text-green-400 mb-1">
              All Checks Passed
            </h3>
            <p className="text-sm text-slate-400">
              Your experiment plan is ready to execute
            </p>
          </div>
        )}
      </div>

      {/* Action Buttons */}
      <div className="px-4 py-3 bg-slate-800 border-t border-slate-700 flex gap-2">
        <button
          onClick={onDryRun}
          className="flex-1 px-4 py-2 bg-slate-700 hover:bg-slate-600 border border-slate-600 rounded text-white text-sm flex items-center justify-center gap-2 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          disabled={plan.actions.length === 0}
        >
          <Zap size={16} />
          Dry Run
        </button>
        <button
          onClick={onRunExperiment}
          disabled={!valid || plan.actions.length === 0}
          className="flex-1 px-4 py-2 bg-green-700 hover:bg-green-600 border border-green-600 rounded text-white text-sm flex items-center justify-center gap-2 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <Play size={16} />
          Run Experiment
        </button>
      </div>
    </div>
  );
}
