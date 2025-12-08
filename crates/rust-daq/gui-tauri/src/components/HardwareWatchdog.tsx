import { useQuery } from '@tanstack/react-query';
import { invoke } from '@tauri-apps/api/tauri';
import {
  Shield,
  Thermometer,
  Gauge,
  Zap,
  Camera,
  Move,
  AlertTriangle,
  CheckCircle,
  Sun,
} from 'lucide-react';
import { DeviceInfo } from '../App';

interface DeviceState {
  device_id: string;
  online: boolean;
  position?: number;
  last_reading?: number;
  armed?: boolean;
  streaming?: boolean;
  exposure_ms?: number;
  temperature?: number;
  wavelength_nm?: number;
  emission_on?: boolean;
  shutter_open?: boolean;
}

interface HardwareWatchdogProps {
  devices: DeviceInfo[];
  compact?: boolean;
}

interface WatchdogAlert {
  deviceId: string;
  deviceName: string;
  severity: 'critical' | 'warning' | 'info';
  message: string;
}

function DeviceMonitor({ device }: { device: DeviceInfo }) {
  const { data: state } = useQuery<DeviceState>({
    queryKey: ['device-state', device.id],
    queryFn: async () => {
      return await invoke<DeviceState>('get_device_state', {
        deviceId: device.id,
      });
    },
    refetchInterval: 1000, // Poll every second for live updates
  });

  const getStatusColor = () => {
    if (!state) return 'bg-slate-600';
    if (!state.online) return 'bg-red-600';
    if (state.streaming) return 'bg-green-600 animate-pulse';
    if (state.armed) return 'bg-yellow-600';
    return 'bg-green-600';
  };

  const getIcon = () => {
    if (device.is_frame_producer) return <Camera className="w-4 h-4" />;
    if (device.is_movable) return <Move className="w-4 h-4" />;
    if (device.is_readable) return <Gauge className="w-4 h-4" />;
    if (device.is_wavelength_tunable) return <Sun className="w-4 h-4" />;
    return <Zap className="w-4 h-4" />;
  };

  return (
    <div className="p-2 bg-slate-800 rounded border border-slate-700">
      <div className="flex items-center justify-between mb-1">
        <div className="flex items-center gap-2">
          {getIcon()}
          <span className="text-xs font-semibold truncate">{device.name}</span>
        </div>
        <div className={`w-2 h-2 rounded-full ${getStatusColor()}`} />
      </div>

      {state && state.online && (
        <div className="space-y-0.5 text-xs">
          {state.position !== undefined && (
            <div className="flex justify-between">
              <span className="text-slate-400">Pos:</span>
              <span className="font-mono">
                {state.position.toFixed(2)} {device.position_units}
              </span>
            </div>
          )}
          {state.last_reading !== undefined && (
            <div className="flex justify-between">
              <span className="text-slate-400">Val:</span>
              <span className="font-mono">
                {state.last_reading.toFixed(3)} {device.reading_units}
              </span>
            </div>
          )}
          {state.temperature !== undefined && (
            <div className="flex justify-between">
              <span className="text-slate-400">Temp:</span>
              <span className={`font-mono ${
                state.temperature > 35 ? 'text-red-400' :
                state.temperature > 30 ? 'text-yellow-400' : 'text-green-400'
              }`}>
                {state.temperature.toFixed(1)}°C
              </span>
            </div>
          )}
          {state.wavelength_nm !== undefined && (
            <div className="flex justify-between">
              <span className="text-slate-400">λ:</span>
              <span className="font-mono">{state.wavelength_nm} nm</span>
            </div>
          )}
          {state.emission_on !== undefined && (
            <div className="flex justify-between">
              <span className="text-slate-400">Emission:</span>
              <span className={state.emission_on ? 'text-green-400' : 'text-slate-500'}>
                {state.emission_on ? 'ON' : 'OFF'}
              </span>
            </div>
          )}
          {state.shutter_open !== undefined && (
            <div className="flex justify-between">
              <span className="text-slate-400">Shutter:</span>
              <span className={state.shutter_open ? 'text-green-400' : 'text-slate-500'}>
                {state.shutter_open ? 'OPEN' : 'CLOSED'}
              </span>
            </div>
          )}
          {state.exposure_ms !== undefined && (
            <div className="flex justify-between">
              <span className="text-slate-400">Exp:</span>
              <span className="font-mono">{state.exposure_ms.toFixed(1)} ms</span>
            </div>
          )}
        </div>
      )}

      {state && !state.online && (
        <div className="text-xs text-red-400">OFFLINE</div>
      )}
    </div>
  );
}

function AlertPanel({ alerts }: { alerts: WatchdogAlert[] }) {
  if (alerts.length === 0) {
    return (
      <div className="p-3 bg-green-900/20 border border-green-800 rounded flex items-center gap-2">
        <CheckCircle className="w-4 h-4 text-green-400" />
        <span className="text-xs text-green-400 font-medium">All systems normal</span>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {alerts.map((alert, i) => {
        const getColor = () => {
          switch (alert.severity) {
            case 'critical':
              return 'bg-red-900/20 border-red-800 text-red-400';
            case 'warning':
              return 'bg-yellow-900/20 border-yellow-800 text-yellow-400';
            case 'info':
              return 'bg-blue-900/20 border-blue-800 text-blue-400';
          }
        };

        return (
          <div key={i} className={`p-2 border rounded ${getColor()}`}>
            <div className="flex items-start gap-2">
              <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-0.5" />
              <div className="flex-1 min-w-0">
                <p className="text-xs font-semibold">{alert.deviceName}</p>
                <p className="text-xs opacity-90 mt-0.5">{alert.message}</p>
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

export default function HardwareWatchdog({ devices, compact = false }: HardwareWatchdogProps) {
  // Query all device states
  const deviceStates = useQuery<DeviceState[]>({
    queryKey: ['all-device-states'],
    queryFn: async () => {
      const states = await Promise.all(
        devices.map((device) =>
          invoke<DeviceState>('get_device_state', { deviceId: device.id }).catch(() => null)
        )
      );
      return states.filter((s): s is DeviceState => s !== null);
    },
    refetchInterval: 1000,
    enabled: devices.length > 0,
  });

  // Generate alerts based on device states
  const alerts: WatchdogAlert[] = [];

  if (deviceStates.data) {
    for (const state of deviceStates.data) {
      const device = devices.find((d) => d.id === state.device_id);
      if (!device) continue;

      if (!state.online) {
        alerts.push({
          deviceId: state.device_id,
          deviceName: device.name,
          severity: 'critical',
          message: 'Device is offline',
        });
      }

      if (state.temperature !== undefined && state.temperature > 35) {
        alerts.push({
          deviceId: state.device_id,
          deviceName: device.name,
          severity: 'critical',
          message: `Temperature critical: ${state.temperature.toFixed(1)}°C`,
        });
      } else if (state.temperature !== undefined && state.temperature > 30) {
        alerts.push({
          deviceId: state.device_id,
          deviceName: device.name,
          severity: 'warning',
          message: `Temperature elevated: ${state.temperature.toFixed(1)}°C`,
        });
      }

      // Check if position is near limits
      if (state.position !== undefined && device.max_position !== undefined) {
        const range = device.max_position - (device.min_position ?? 0);
        const distanceToMax = device.max_position - state.position;
        const distanceToMin = state.position - (device.min_position ?? 0);

        if (distanceToMax < range * 0.05 || distanceToMin < range * 0.05) {
          alerts.push({
            deviceId: state.device_id,
            deviceName: device.name,
            severity: 'warning',
            message: `Position near limit: ${state.position.toFixed(2)} ${device.position_units}`,
          });
        }
      }
    }
  }

  if (compact) {
    return (
      <div className="p-3 bg-slate-800 border border-slate-700 rounded">
        <div className="flex items-center gap-2 mb-2">
          <Shield className="w-4 h-4 text-blue-400" />
          <span className="text-sm font-semibold">Hardware Status</span>
        </div>
        <AlertPanel alerts={alerts} />
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col bg-slate-900">
      {/* Header */}
      <div className="px-4 py-3 bg-slate-800 border-b border-slate-700">
        <div className="flex items-center gap-2 mb-1">
          <Shield className="w-5 h-5 text-blue-400" />
          <h2 className="text-lg font-bold">Hardware Watchdog</h2>
        </div>
        <p className="text-xs text-slate-400">Live device monitoring</p>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {/* Alerts */}
        <div>
          <h3 className="text-sm font-semibold text-slate-300 mb-2 flex items-center gap-2">
            <AlertTriangle className="w-4 h-4" />
            Alerts
          </h3>
          <AlertPanel alerts={alerts} />
        </div>

        {/* Device Grid */}
        <div>
          <h3 className="text-sm font-semibold text-slate-300 mb-2 flex items-center gap-2">
            <Thermometer className="w-4 h-4" />
            Device Status ({devices.length})
          </h3>
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-2">
            {devices.map((device) => (
              <DeviceMonitor key={device.id} device={device} />
            ))}
          </div>
        </div>
      </div>

      {/* Footer Stats */}
      <div className="px-4 py-2 bg-slate-800 border-t border-slate-700 flex justify-between text-xs text-slate-400">
        <span>
          {deviceStates.data?.filter((s) => s.online).length ?? 0}/{devices.length} online
        </span>
        <span>
          {alerts.filter((a) => a.severity === 'critical').length} critical,{' '}
          {alerts.filter((a) => a.severity === 'warning').length} warnings
        </span>
      </div>
    </div>
  );
}
