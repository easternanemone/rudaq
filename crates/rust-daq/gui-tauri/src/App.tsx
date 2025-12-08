import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import ConnectionStatus from "./components/ConnectionStatus";
import DeviceStatusPanel from "./components/DeviceStatusPanel";
import ManualControlPanel from "./components/ManualControlPanel";
import ExperimentSequencer from "./components/ExperimentSequencer";
import { invoke } from "@tauri-apps/api/tauri";
import { LayoutDashboard, FlaskConical } from "lucide-react";

export interface DeviceInfo {
  id: string;
  name: string;
  driver_type: string;
  is_movable: boolean;
  is_readable: boolean;
  is_triggerable: boolean;
  is_frame_producer: boolean;
  is_exposure_controllable: boolean;
  is_shutter_controllable: boolean;
  is_wavelength_tunable: boolean;
  is_emission_controllable: boolean;
  position_units?: string;
  min_position?: number;
  max_position?: number;
  reading_units?: string;
}

function App() {
  const [connected, setConnected] = useState(false);
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null);
  const [activeView, setActiveView] = useState<'dashboard' | 'sequencer'>('dashboard');

  // Query devices list
  const {
    data: devices,
    isLoading,
    error,
    refetch,
  } = useQuery<DeviceInfo[]>({
    queryKey: ["devices"],
    queryFn: async () => {
      return await invoke<DeviceInfo[]>("list_devices");
    },
    enabled: connected,
    refetchInterval: connected ? 5000 : false, // Refresh every 5 seconds when connected
  });

  const handleConnect = async (address: string) => {
    try {
      await invoke("connect_to_daemon", { address });
      setConnected(true);
      refetch();
    } catch (error) {
      console.error("Connection error:", error);
      throw error;
    }
  };

  const selectedDeviceInfo = devices?.find((d) => d.id === selectedDevice);

  return (
    <div className="h-screen bg-slate-900 text-white flex flex-col">
      {/* Header */}
      <header className="bg-slate-800 border-b border-slate-700 px-6 py-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-6">
            <div>
              <h1 className="text-2xl font-bold text-white">rust-daq GUI</h1>
              <p className="text-sm text-slate-400">Hardware Control Dashboard</p>
            </div>
            
            {/* Tab Navigation */}
            <div className="flex gap-2 ml-8">
              <button
                onClick={() => setActiveView('dashboard')}
                className={`
                  px-4 py-2 rounded-lg flex items-center gap-2 transition-colors text-sm font-medium
                  ${activeView === 'dashboard' 
                    ? 'bg-blue-600 text-white' 
                    : 'bg-slate-700 text-slate-300 hover:bg-slate-600'}
                `}
              >
                <LayoutDashboard size={16} />
                Dashboard
              </button>
              <button
                onClick={() => setActiveView('sequencer')}
                className={`
                  px-4 py-2 rounded-lg flex items-center gap-2 transition-colors text-sm font-medium
                  ${activeView === 'sequencer' 
                    ? 'bg-blue-600 text-white' 
                    : 'bg-slate-700 text-slate-300 hover:bg-slate-600'}
                `}
              >
                <FlaskConical size={16} />
                Sequencer
              </button>
            </div>
          </div>
          <ConnectionStatus connected={connected} onConnect={handleConnect} />
        </div>
      </header>

      {/* Main Content */}
      {activeView === 'sequencer' ? (
        <ExperimentSequencer />
      ) : (
        <div className="flex-1 flex overflow-hidden">
          {!connected ? (
            <div className="flex-1 flex items-center justify-center">
              <div className="text-center">
                <div className="text-6xl mb-4">🔌</div>
                <h2 className="text-2xl font-semibold mb-2">Not Connected</h2>
                <p className="text-slate-400">
                  Connect to the daemon to start controlling devices
                </p>
              </div>
            </div>
          ) : isLoading ? (
            <div className="flex-1 flex items-center justify-center">
              <div className="text-center">
                <div className="animate-spin text-4xl mb-4">⚙️</div>
                <p className="text-slate-400">Loading devices...</p>
              </div>
            </div>
          ) : error ? (
            <div className="flex-1 flex items-center justify-center">
              <div className="text-center">
                <div className="text-6xl mb-4">⚠️</div>
                <h2 className="text-2xl font-semibold mb-2 text-red-400">
                  Error Loading Devices
                </h2>
                <p className="text-slate-400">{String(error)}</p>
                <button
                  onClick={() => refetch()}
                  className="btn-primary mt-4"
                >
                  Retry
                </button>
              </div>
            </div>
          ) : devices && devices.length === 0 ? (
            <div className="flex-1 flex items-center justify-center">
              <div className="text-center">
                <div className="text-6xl mb-4">📭</div>
                <h2 className="text-2xl font-semibold mb-2">No Devices Found</h2>
                <p className="text-slate-400">
                  No hardware devices are currently registered with the daemon
                </p>
              </div>
            </div>
          ) : (
            <>
              {/* Left Panel - Device List */}
              <div className="w-80 bg-slate-800 border-r border-slate-700 overflow-y-auto">
                <DeviceStatusPanel
                  devices={devices!}
                  selectedDevice={selectedDevice}
                  onSelectDevice={setSelectedDevice}
                />
              </div>

              {/* Right Panel - Device Control */}
              <div className="flex-1 overflow-y-auto p-6">
                {selectedDeviceInfo ? (
                  <ManualControlPanel device={selectedDeviceInfo} />
                ) : (
                  <div className="h-full flex items-center justify-center">
                    <div className="text-center">
                      <div className="text-6xl mb-4">👈</div>
                      <h2 className="text-2xl font-semibold mb-2">
                        Select a Device
                      </h2>
                      <p className="text-slate-400">
                        Choose a device from the list to control it
                      </p>
                    </div>
                  </div>
                )}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

export default App;
