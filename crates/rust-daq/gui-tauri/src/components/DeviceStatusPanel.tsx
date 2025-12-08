import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/tauri";
import {
  Move,
  Gauge,
  Camera,
  Zap,
  Sun,
  Circle,
} from "lucide-react";
import type { DeviceInfo } from "../App";

interface DeviceState {
  device_id: string;
  online: boolean;
  position?: number;
  last_reading?: number;
  armed?: boolean;
  streaming?: boolean;
  exposure_ms?: number;
}

interface DeviceStatusPanelProps {
  devices: DeviceInfo[];
  selectedDevice: string | null;
  onSelectDevice: (deviceId: string) => void;
}

function DeviceCard({
  device,
  selected,
  onClick,
}: {
  device: DeviceInfo;
  selected: boolean;
  onClick: () => void;
}) {
  // Query device state
  const { data: state } = useQuery<DeviceState>({
    queryKey: ["device-state", device.id],
    queryFn: async () => {
      return await invoke<DeviceState>("get_device_state", {
        deviceId: device.id,
      });
    },
    refetchInterval: 1000, // Poll every second
  });

  const getDeviceIcon = () => {
    if (device.is_frame_producer) return <Camera className="w-5 h-5" />;
    if (device.is_movable) return <Move className="w-5 h-5" />;
    if (device.is_readable) return <Gauge className="w-5 h-5" />;
    if (device.is_triggerable) return <Zap className="w-5 h-5" />;
    return <Circle className="w-5 h-5" />;
  };

  const getStatusColor = () => {
    if (!state) return "bg-slate-600";
    if (!state.online) return "bg-red-600";
    if (state.streaming) return "bg-green-600 animate-pulse";
    if (state.armed) return "bg-yellow-600";
    return "bg-green-600";
  };

  return (
    <div
      onClick={onClick}
      className={`p-4 border-b border-slate-700 cursor-pointer transition-colors ${
        selected
          ? "bg-primary-900 border-l-4 border-l-primary-500"
          : "hover:bg-slate-700"
      }`}
    >
      <div className="flex items-start justify-between mb-2">
        <div className="flex items-center gap-2">
          {getDeviceIcon()}
          <div>
            <h3 className="font-semibold">{device.name}</h3>
            <p className="text-xs text-slate-400">{device.driver_type}</p>
          </div>
        </div>
        <div className={`w-3 h-3 rounded-full ${getStatusColor()}`} />
      </div>

      {state && (
        <div className="mt-2 space-y-1 text-sm">
          {state.position !== undefined && (
            <div className="flex justify-between">
              <span className="text-slate-400">Position:</span>
              <span className="font-mono">
                {state.position.toFixed(3)} {device.position_units || ""}
              </span>
            </div>
          )}
          {state.last_reading !== undefined && (
            <div className="flex justify-between">
              <span className="text-slate-400">Reading:</span>
              <span className="font-mono">
                {state.last_reading.toFixed(3)} {device.reading_units || ""}
              </span>
            </div>
          )}
          {state.exposure_ms !== undefined && (
            <div className="flex justify-between">
              <span className="text-slate-400">Exposure:</span>
              <span className="font-mono">{state.exposure_ms.toFixed(1)} ms</span>
            </div>
          )}
        </div>
      )}

      <div className="mt-2 flex flex-wrap gap-1">
        {device.is_movable && (
          <span className="text-xs px-2 py-0.5 bg-blue-900 text-blue-200 rounded">
            Movable
          </span>
        )}
        {device.is_readable && (
          <span className="text-xs px-2 py-0.5 bg-green-900 text-green-200 rounded">
            Readable
          </span>
        )}
        {device.is_frame_producer && (
          <span className="text-xs px-2 py-0.5 bg-purple-900 text-purple-200 rounded">
            Camera
          </span>
        )}
        {device.is_wavelength_tunable && (
          <span className="text-xs px-2 py-0.5 bg-yellow-900 text-yellow-200 rounded">
            Tunable
          </span>
        )}
      </div>
    </div>
  );
}

export default function DeviceStatusPanel({
  devices,
  selectedDevice,
  onSelectDevice,
}: DeviceStatusPanelProps) {
  return (
    <div className="h-full">
      <div className="p-4 border-b border-slate-700">
        <h2 className="text-lg font-bold flex items-center gap-2">
          <Sun className="w-5 h-5" />
          Devices ({devices.length})
        </h2>
        <p className="text-xs text-slate-400 mt-1">
          Click a device to control it
        </p>
      </div>
      <div className="overflow-y-auto">
        {devices.map((device) => (
          <DeviceCard
            key={device.id}
            device={device}
            selected={selectedDevice === device.id}
            onClick={() => onSelectDevice(device.id)}
          />
        ))}
      </div>
    </div>
  );
}
