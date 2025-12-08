import { useState, useEffect } from 'react';
import Editor from '@monaco-editor/react';
import { ActionBlock } from '../types/experiment';
import { generateRhaiScript, generatePythonScript } from '../utils/scriptGenerator';
import { Download, Code } from 'lucide-react';
import { save } from '@tauri-apps/api/dialog';
import { writeTextFile } from '@tauri-apps/api/fs';

interface ScriptPreviewProps {
  actions: ActionBlock[];
  onEjectToScript?: (script: string) => void;
}

type ScriptLanguage = 'rhai' | 'python';

function ScriptPreview({ actions, onEjectToScript }: ScriptPreviewProps) {
  const [language, setLanguage] = useState<ScriptLanguage>('rhai');
  const [script, setScript] = useState('');

  useEffect(() => {
    const generated =
      language === 'rhai'
        ? generateRhaiScript(actions)
        : generatePythonScript(actions);
    setScript(generated);
  }, [actions, language]);

  const handleExport = async () => {
    const extension = language === 'rhai' ? 'rhai' : 'py';
    const filters = [
      {
        name: language === 'rhai' ? 'Rhai Script' : 'Python Script',
        extensions: [extension],
      },
    ];

    const filePath = await save({
      filters,
      defaultPath: `experiment.${extension}`,
    });

    if (filePath) {
      await writeTextFile(filePath, script);
    }
  };

  return (
    <div className="h-full flex flex-col bg-slate-800">
      <div className="px-4 py-3 border-b border-slate-700 flex items-center justify-between">
        <div>
          <h3 className="font-semibold text-white">Script Preview</h3>
          <p className="text-xs text-slate-400 mt-0.5">
            Real-time generated code
          </p>
        </div>
        <div className="flex items-center gap-2">
          <select
            value={language}
            onChange={(e) => setLanguage(e.target.value as ScriptLanguage)}
            className="px-3 py-1.5 bg-slate-700 border border-slate-600 rounded text-white text-sm focus:outline-none focus:border-blue-500"
          >
            <option value="rhai">Rhai</option>
            <option value="python">Python</option>
          </select>
          <button
            onClick={handleExport}
            className="px-3 py-1.5 bg-slate-700 hover:bg-slate-600 border border-slate-600 rounded text-white text-sm flex items-center gap-1.5 transition-colors"
            title="Export script"
          >
            <Download size={14} />
            Export
          </button>
          {onEjectToScript && (
            <button
              onClick={() => onEjectToScript(script)}
              className="px-3 py-1.5 bg-blue-600 hover:bg-blue-700 rounded text-white text-sm flex items-center gap-1.5 transition-colors"
              title="Switch to code editor mode"
            >
              <Code size={14} />
              Edit Code
            </button>
          )}
        </div>
      </div>

      <div className="flex-1 overflow-hidden">
        <Editor
          height="100%"
          language={language === 'rhai' ? 'javascript' : 'python'}
          value={script}
          theme="vs-dark"
          options={{
            readOnly: true,
            minimap: { enabled: false },
            fontSize: 13,
            lineNumbers: 'on',
            scrollBeyondLastLine: false,
            automaticLayout: true,
            tabSize: 4,
          }}
        />
      </div>
    </div>
  );
}

export default ScriptPreview;
