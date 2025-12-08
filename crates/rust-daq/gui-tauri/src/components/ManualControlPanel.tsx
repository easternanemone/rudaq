import { useState } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import { useQuery, useMutation } from "@tanstack/react-query";
import {
  Move,
  StopCircle,
  Gauge,
  Camera,
  Sun,
  AlertCircle,
  CheckCircle,
  Loader2,
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

interface ManualControlPanelProps {
  device: DeviceInfo;
}

function MovableControl({ device }: { device: DeviceInfo }) {
  const [targetPosition, setTargetPosition] = useState("");
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);

  const moveMutation = useMutation({
    mutationFn: async ({ position, wait }: { position: number; wait: boolean }) => {
      return await invoke<string>("move_absolute", {
        deviceId: device.id,
        position,
        waitForCompletion: wait,
      });
    },
    onSuccess: (msg) => {
      setMessage({ type: "success", text: msg });
      setTimeout(() => setMessage(null), 3000);
    },
    onError: (error: string) => {
      setMessage({ type: "error", text: error });
      setTimeout(() => setMessage(null), 5000);
    },
  });

  const stopMutation = useMutation({
    mutationFn: async () => {
      return await invoke<string>("stop_motion", { deviceId: device.id });
    },
    onSuccess: (msg) => {
      setMessage({ type: "success", text: msg });
    },
    onError: (error: string) => {
      setMessage({ type: "error", text: error });
    },
  });

  const handleMove = (wait: boolean) => {
    const pos = parseFloat(targetPosition);
    if (isNaN(pos)) {
      setMessage({ type: "error", text: "Invalid position value" });
      return;
    }
    if (device.min_position !== undefined && pos < device.min_position) {
      setMessage({
        type: "error",
        text: `Position below minimum (${device.min_position})`,
      });
      return;
    }
    if (device.max_position !== undefined && pos > device.max_position) {
      setMessage({
        type: "error",
        text: `Position above maximum (${device.max_position})`,
      });
      return;
    }
    moveMutation.mutate({ position: pos, wait });
  };

  return (
    <div className="space-y-4">
      <h3 className="text-lg font-semibold flex items-center gap-2">
        <Move className="w-5 h-5" />
        Motion Control
      </h3>

      <div>
        <label className="label">
          Target Position {device.position_units && `(${device.position_units})`}
        </label>
        <input
          type="number"
          step="0.001"
          value={targetPosition}
          onChange={(e) => setTargetPosition(e.target.value)}
          className="input w-full"
          placeholder="Enter position..."
          disabled={moveMutation.isPending}
        />
        {device.min_position !== undefined && device.max_position !== undefined && (
          <p className="text-xs text-slate-400 mt-1">
            Range: {device.min_position} to {device.max_position}
          </p>
        )}
      </div>

      <div className="flex gap-2">
        <button
          onClick={() => handleMove(false)}
          disabled={moveMutation.isPending || !targetPosition}
          className="btn-primary flex-1"
        >
          {moveMutation.isPending ? (
            <span className="flex items-center gap-2">
              <Loader2 className="w-4 h-4 animate-spin" />
              Moving...
            </span>
          ) : (
            "Move (Async)"
          )}
        </button>
        <button
          onClick={() => handleMove(true)}
          disabled={moveMutation.isPending || !targetPosition}
          className="btn-secondary flex-1"
        >
          Move & Wait
        </button>
        <button
          onClick={() => stopMutation.mutate()}
          disabled={stopMutation.isPending}
          className="btn-danger"
        >
          <StopCircle className="w-4 h-4" />
        </button>
      </div>

      {message && (
        <div
          className={`flex items-center gap-2 px-4 py-3 rounded-lg ${
            message.type === "success"
              ? "bg-green-900 border border-green-700 text-green-200"
              : "bg-red-900 border border-red-700 text-red-200"
          }`}
        >
          {message.type === "success" ? (
            <CheckCircle className="w-5 h-5" />
          ) : (
            <AlertCircle className="w-5 h-5" />
          )}
          <span className="text-sm">{message.text}</span>
        </div>
      )}
    </div>
  );
}

function ReadableControl({ device }: { device: DeviceInfo }) {
  const [lastReading, setLastReading] = useState<{ value: number; units: string } | null>(
    null
  );

  const readMutation = useMutation({
    mutationFn: async () => {
      const [value, units] = await invoke<[number, string]>("read_value", {
        deviceId: device.id,
      });
      return { value, units };
    },
    onSuccess: (data) => {
      setLastReading(data);
    },
  });

  return (
    <div className="space-y-4">
      <h3 className="text-lg font-semibold flex items-center gap-2">
        <Gauge className="w-5 h-5" />
        Scalar Readout
      </h3>

      <button
        onClick={() => readMutation.mutate()}
        disabled={readMutation.isPending}
        className="btn-primary w-full"
      >
        {readMutation.isPending ? (
          <span className="flex items-center gap-2 justify-center">
            <Loader2 className="w-4 h-4 animate-spin" />
            Reading...
          </span>
        ) : (
          "Read Value"
        )}
      </button>

      {lastReading && (
        <div className="card p-4">
          <p className="text-sm text-slate-400 mb-1">Latest Reading</p>
          <p className="text-3xl font-mono font-bold">
            {lastReading.value.toFixed(6)}
            <span className="text-lg text-slate-400 ml-2">{lastReading.units}</span>
          </p>
        </div>
      )}

      {readMutation.isError && (
        <div className="bg-red-900 border border-red-700 text-red-200 px-4 py-3 rounded">
          <p className="text-sm">{String(readMutation.error)}</p>
        </div>
      )}
    </div>
  );
}

function ExposureControl({ device }: { device: DeviceInfo }) {
  const [exposureMs, setExposureMs] = useState("");

  const setExposureMutation = useMutation({
    mutationFn: async (ms: number) => {
      return await invoke<number>("set_exposure", {
        deviceId: device.id,
        exposureMs: ms,
      });
    },
  });

  const { data: currentExposure } = useQuery({
    queryKey: ["exposure", device.id],
    queryFn: async () => {
      return await invoke<number>("get_exposure", { deviceId: device.id });
    },
    refetchInterval: 2000,
  });

  const handleSetExposure = () => {
    const ms = parseFloat(exposureMs);
    if (!isNaN(ms) && ms > 0) {
      setExposureMutation.mutate(ms);
    }
  };

  return (
    <div className="space-y-4">
      <h3 className="text-lg font-semibold flex items-center gap-2">
        <Camera className="w-5 h-5" />
        Exposure Control
      </h3>

      {currentExposure !== undefined && (
        <div className="card p-4">
          <p className="text-sm text-slate-400 mb-1">Current Exposure</p>
          <p className="text-2xl font-mono font-bold">
            {currentExposure.toFixed(2)} <span className="text-lg text-slate-400">ms</span>
          </p>
        </div>
      )}

      <div>
        <label className="label">New Exposure (ms)</label>
        <div className="flex gap-2">
          <input
            type="number"
            step="0.1"
            min="0"
            value={exposureMs}
            onChange={(e) => setExposureMs(e.target.value)}
            className="input flex-1"
            placeholder="Enter exposure time..."
            onKeyDown={(e) => e.key === "Enter" && handleSetExposure()}
          />
          <button
            onClick={handleSetExposure}
            disabled={setExposureMutation.isPending || !exposureMs}
            className="btn-primary"
          >
            Set
          </button>
        </div>
      </div>

      {setExposureMutation.isError && (
        <div className="bg-red-900 border border-red-700 text-red-200 px-4 py-3 rounded">
          <p className="text-sm">{String(setExposureMutation.error)}</p>
        </div>
      )}
    </div>
  );
}

function LaserControl({ device }: { device: DeviceInfo }) {
  const [wavelengthNm, setWavelengthNm] = useState("");

  const setShutterMutation = useMutation({
    mutationFn: async (open: boolean) => {
      return await invoke<boolean>("set_shutter", {
        deviceId: device.id,
        open,
      });
    },
  });

  const setWavelengthMutation = useMutation({
    mutationFn: async (nm: number) => {
      return await invoke<number>("set_wavelength", {
        deviceId: device.id,
        wavelengthNm: nm,
      });
    },
  });

  const setEmissionMutation = useMutation({
    mutationFn: async (enabled: boolean) => {
      return await invoke<boolean>("set_emission", {
        deviceId: device.id,
        enabled,
      });
    },
  });

  return (
    <div className="space-y-4">
      <h3 className="text-lg font-semibold flex items-center gap-2">
        <Sun className="w-5 h-5" />
        Laser Control
      </h3>

      {device.is_shutter_controllable && (
        <div>
          <label className="label">Shutter Control</label>
          <div className="flex gap-2">
            <button
              onClick={() => setShutterMutation.mutate(true)}
              disabled={setShutterMutation.isPending}
              className="btn-primary flex-1"
            >
              Open Shutter
            </button>
            <button
              onClick={() => setShutterMutation.mutate(false)}
              disabled={setShutterMutation.isPending}
              className="btn-secondary flex-1"
            >
              Close Shutter
            </button>
          </div>
        </div>
      )}

      {device.is_wavelength_tunable && (
        <div>
          <label className="label">Wavelength (nm)</label>
          <div className="flex gap-2">
            <input
              type="number"
              step="1"
              value={wavelengthNm}
              onChange={(e) => setWavelengthNm(e.target.value)}
              className="input flex-1"
              placeholder="Enter wavelength..."
            />
            <button
              onClick={() => {
                const nm = parseFloat(wavelengthNm);
                if (!isNaN(nm)) setWavelengthMutation.mutate(nm);
              }}
              disabled={setWavelengthMutation.isPending || !wavelengthNm}
              className="btn-primary"
            >
              Set
            </button>
          </div>
        </div>
      )}

      {device.is_emission_controllable && (
        <div>
          <label className="label">Emission Control</label>
          <div className="flex gap-2">
            <button
              onClick={() => setEmissionMutation.mutate(true)}
              disabled={setEmissionMutation.isPending}
              className="btn-primary flex-1"
            >
              Enable Emission
            </button>
            <button
              onClick={() => setEmissionMutation.mutate(false)}
              disabled={setEmissionMutation.isPending}
              className="btn-danger flex-1"
            >
              Disable Emission
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export default function ManualControlPanel({ device }: ManualControlPanelProps) {
  const { data: state } = useQuery<DeviceState>({
    queryKey: ["device-state", device.id],
    queryFn: async () => {
      return await invoke<DeviceState>("get_device_state", { deviceId: device.id });
    },
    refetchInterval: 1000,
  });

  const hasLaserControls =
    device.is_shutter_controllable ||
    device.is_wavelength_tunable ||
    device.is_emission_controllable;

  return (
    <div className="space-y-6">
      {/* Device Header */}
      <div className="card p-6">
        <h2 className="text-2xl font-bold mb-2">{device.name}</h2>
        <p className="text-slate-400">Driver: {device.driver_type}</p>
        <p className="text-slate-400">Device ID: {device.id}</p>

        {state && (
          <div className="mt-4 flex items-center gap-2">
            <div
              className={`w-3 h-3 rounded-full ${
                state.online ? "bg-green-500" : "bg-red-500"
              }`}
            />
            <span className={state.online ? "text-green-400" : "text-red-400"}>
              {state.online ? "Online" : "Offline"}
            </span>
          </div>
        )}
      </div>

      {/* Control Panels */}
      <div className="grid grid-cols-1 gap-6">
        {device.is_movable && (
          <div className="card p-6">
            <MovableControl device={device} />
          </div>
        )}

        {device.is_readable && (
          <div className="card p-6">
            <ReadableControl device={device} />
          </div>
        )}

        {device.is_exposure_controllable && (
          <div className="card p-6">
            <ExposureControl device={device} />
          </div>
        )}

        {hasLaserControls && (
          <div className="card p-6">
            <LaserControl device={device} />
          </div>
        )}
      </div>
    </div>
  );
}
