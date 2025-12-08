import { useState } from "react";
import { Wifi, WifiOff } from "lucide-react";

interface ConnectionStatusProps {
  connected: boolean;
  onConnect: (address: string) => Promise<void>;
}

export default function ConnectionStatus({
  connected,
  onConnect,
}: ConnectionStatusProps) {
  const [showDialog, setShowDialog] = useState(false);
  const [address, setAddress] = useState("localhost:50051");
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleConnect = async () => {
    setConnecting(true);
    setError(null);
    try {
      await onConnect(address);
      setShowDialog(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setConnecting(false);
    }
  };

  return (
    <>
      <button
        onClick={() => setShowDialog(true)}
        className={`flex items-center gap-2 px-4 py-2 rounded-lg font-medium ${
          connected
            ? "bg-green-600 hover:bg-green-700"
            : "bg-slate-700 hover:bg-slate-600"
        }`}
      >
        {connected ? (
          <>
            <Wifi className="w-5 h-5" />
            <span>Connected</span>
          </>
        ) : (
          <>
            <WifiOff className="w-5 h-5" />
            <span>Connect to Daemon</span>
          </>
        )}
      </button>

      {showDialog && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="card p-6 w-96">
            <h2 className="text-xl font-bold mb-4">Connect to Daemon</h2>
            <div className="mb-4">
              <label className="label">Daemon Address</label>
              <input
                type="text"
                value={address}
                onChange={(e) => setAddress(e.target.value)}
                placeholder="localhost:50051"
                className="input w-full"
                onKeyDown={(e) => e.key === "Enter" && handleConnect()}
                disabled={connecting}
              />
              <p className="text-xs text-slate-400 mt-1">
                Format: host:port (e.g., localhost:50051)
              </p>
            </div>

            {error && (
              <div className="bg-red-900 border border-red-700 text-red-200 px-4 py-3 rounded mb-4">
                <p className="text-sm">{error}</p>
              </div>
            )}

            <div className="flex gap-2">
              <button
                onClick={handleConnect}
                disabled={connecting}
                className="btn-primary flex-1"
              >
                {connecting ? "Connecting..." : "Connect"}
              </button>
              <button
                onClick={() => setShowDialog(false)}
                disabled={connecting}
                className="btn-secondary"
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
