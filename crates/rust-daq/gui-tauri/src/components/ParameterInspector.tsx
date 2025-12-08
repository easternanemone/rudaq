import { ActionBlock, ACTION_TEMPLATES, ParamDefinition } from '../types/experiment';
import { useQuery } from '@tanstack/react-query';
import { invoke } from '@tauri-apps/api/tauri';
import { DeviceInfo } from '../App';

interface ParameterInspectorProps {
  block: ActionBlock | null;
  onUpdateParams: (blockId: string, params: Record<string, any>) => void;
}

function ParameterInspector({ block, onUpdateParams }: ParameterInspectorProps) {
  const { data: devices } = useQuery<DeviceInfo[]>({
    queryKey: ['devices'],
    queryFn: async () => {
      return await invoke<DeviceInfo[]>('list_devices');
    },
  });

  if (!block) {
    return (
      <div className="h-full flex flex-col bg-slate-800">
        <div className="px-4 py-3 border-b border-slate-700">
          <h3 className="font-semibold text-white">Parameters</h3>
        </div>
        <div className="flex-1 flex items-center justify-center">
          <div className="text-center text-slate-500">
            <div className="text-4xl mb-2">⚙️</div>
            <p className="text-sm">Select an action to edit parameters</p>
          </div>
        </div>
      </div>
    );
  }

  const template = ACTION_TEMPLATES.find((t) => t.type === block.type);

  const handleParamChange = (paramName: string, value: any) => {
    const newParams = { ...block.params, [paramName]: value };
    onUpdateParams(block.id, newParams);
  };

  const renderParamInput = (param: ParamDefinition) => {
    const value = block.params[param.name] ?? param.default;

    switch (param.type) {
      case 'string':
        return (
          <input
            type="text"
            value={value || ''}
            onChange={(e) => handleParamChange(param.name, e.target.value)}
            className="w-full px-3 py-2 bg-slate-700 border border-slate-600 rounded text-white text-sm focus:outline-none focus:border-blue-500"
            placeholder={param.default}
          />
        );

      case 'number':
        return (
          <div className="flex items-center gap-2">
            <input
              type="number"
              value={value ?? ''}
              onChange={(e) =>
                handleParamChange(
                  param.name,
                  e.target.value === '' ? undefined : parseFloat(e.target.value)
                )
              }
              min={param.min}
              max={param.max}
              step="any"
              className="flex-1 px-3 py-2 bg-slate-700 border border-slate-600 rounded text-white text-sm focus:outline-none focus:border-blue-500"
              placeholder={param.default?.toString()}
            />
            {param.unit && (
              <span className="text-sm text-slate-400">{param.unit}</span>
            )}
          </div>
        );

      case 'boolean':
        return (
          <label className="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={value ?? param.default ?? false}
              onChange={(e) => handleParamChange(param.name, e.target.checked)}
              className="w-4 h-4 rounded border-slate-600 bg-slate-700 text-blue-500 focus:ring-2 focus:ring-blue-500"
            />
            <span className="text-sm text-slate-300">
              {value ? 'Enabled' : 'Disabled'}
            </span>
          </label>
        );

      case 'device':
        return (
          <select
            value={value || ''}
            onChange={(e) => handleParamChange(param.name, e.target.value)}
            className="w-full px-3 py-2 bg-slate-700 border border-slate-600 rounded text-white text-sm focus:outline-none focus:border-blue-500"
          >
            <option value="">Select device...</option>
            {devices?.map((device) => (
              <option key={device.id} value={device.id}>
                {device.name} ({device.driver_type})
              </option>
            ))}
          </select>
        );

      case 'select':
        return (
          <select
            value={value || ''}
            onChange={(e) => handleParamChange(param.name, e.target.value)}
            className="w-full px-3 py-2 bg-slate-700 border border-slate-600 rounded text-white text-sm focus:outline-none focus:border-blue-500"
          >
            <option value="">Select...</option>
            {param.options?.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        );

      default:
        return <div className="text-xs text-slate-500">Unknown type</div>;
    }
  };

  return (
    <div className="h-full flex flex-col bg-slate-800">
      <div className="px-4 py-3 border-b border-slate-700">
        <div className="flex items-center gap-2">
          <span className="text-xl">{template?.icon}</span>
          <div>
            <h3 className="font-semibold text-white">{template?.label}</h3>
            <p className="text-xs text-slate-400">{template?.description}</p>
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {template?.paramSchema.map((param) => (
          <div key={param.name}>
            <label className="block mb-1.5">
              <div className="flex items-center gap-1">
                <span className="text-sm font-medium text-slate-300">
                  {param.label}
                </span>
                {param.required && (
                  <span className="text-red-400 text-xs">*</span>
                )}
              </div>
            </label>
            {renderParamInput(param)}
          </div>
        ))}

        {template?.paramSchema.length === 0 && (
          <div className="text-center text-slate-500 py-4">
            <p className="text-sm">No parameters to configure</p>
          </div>
        )}
      </div>

      <div className="px-4 py-3 border-t border-slate-700">
        <div className="text-xs text-slate-500">
          <div className="mb-1">
            <span className="font-medium">Action ID:</span> {block.id}
          </div>
          <div>
            <span className="font-medium">Type:</span> {block.type}
          </div>
        </div>
      </div>
    </div>
  );
}

export default ParameterInspector;
