import { ActionBlock } from '../types/experiment';

export function generateRhaiScript(actions: ActionBlock[]): string {
  const lines: string[] = [];
  lines.push('// Generated experiment script');
  lines.push('// Edit this script or use it as a starting point');
  lines.push('');

  const generateActionCode = (block: ActionBlock, indent: number = 0): string[] => {
    const indentStr = '    '.repeat(indent);
    const code: string[] = [];

    switch (block.type) {
      case 'move_absolute':
        code.push(
          `${indentStr}move_absolute("${block.params.device}", ${block.params.position}, ${block.params.wait ?? true});`
        );
        break;

      case 'move_relative':
        code.push(
          `${indentStr}move_relative("${block.params.device}", ${block.params.distance}, ${block.params.wait ?? true});`
        );
        break;

      case 'set_parameter':
        if (block.params.parameter === 'exposure') {
          code.push(
            `${indentStr}set_exposure("${block.params.device}", ${block.params.value});`
          );
        } else if (block.params.parameter === 'wavelength') {
          code.push(
            `${indentStr}set_wavelength("${block.params.device}", ${block.params.value});`
          );
        } else {
          code.push(
            `${indentStr}set_parameter("${block.params.device}", "${block.params.parameter}", ${block.params.value});`
          );
        }
        break;

      case 'trigger':
        if (block.params.count > 1) {
          code.push(
            `${indentStr}for _ in 0..${block.params.count} {`
          );
          code.push(
            `${indentStr}    trigger("${block.params.device}");`
          );
          code.push(`${indentStr}}`);
        } else {
          code.push(
            `${indentStr}trigger("${block.params.device}");`
          );
        }
        break;

      case 'read':
        code.push(
          `${indentStr}let ${block.params.variable} = read("${block.params.device}");`
        );
        break;

      case 'loop':
        code.push(
          `${indentStr}for ${block.params.variable} in 0..${block.params.iterations} {`
        );
        if (block.children) {
          for (const child of block.children) {
            code.push(...generateActionCode(child, indent + 1));
          }
        }
        code.push(`${indentStr}}`);
        break;

      case 'delay':
        code.push(
          `${indentStr}sleep(${block.params.duration});`
        );
        break;

      case 'parallel':
        code.push(`${indentStr}// Parallel execution`);
        code.push(`${indentStr}// Note: Rhai doesn't support true parallelism`);
        code.push(`${indentStr}// These actions will run sequentially:`);
        if (block.children) {
          for (const child of block.children) {
            code.push(...generateActionCode(child, indent));
          }
        }
        break;

      default:
        code.push(`${indentStr}// Unknown action: ${block.type}`);
    }

    return code;
  };

  for (const action of actions) {
    lines.push(...generateActionCode(action));
  }

  if (lines.length === 3) {
    lines.push('// No actions defined');
  }

  return lines.join('\n');
}

export function generatePythonScript(actions: ActionBlock[]): string {
  const lines: string[] = [];
  lines.push('# Generated experiment script');
  lines.push('# Requires: rust-daq Python client');
  lines.push('from rust_daq import DaqClient');
  lines.push('');
  lines.push('# Connect to daemon');
  lines.push('client = DaqClient("localhost:50051")');
  lines.push('');

  const generateActionCode = (block: ActionBlock, indent: number = 0): string[] => {
    const indentStr = '    '.repeat(indent);
    const code: string[] = [];

    switch (block.type) {
      case 'move_absolute':
        code.push(
          `${indentStr}client.move_absolute("${block.params.device}", ${block.params.position}, wait=${block.params.wait ? 'True' : 'False'})`
        );
        break;

      case 'move_relative':
        code.push(
          `${indentStr}client.move_relative("${block.params.device}", ${block.params.distance}, wait=${block.params.wait ? 'True' : 'False'})`
        );
        break;

      case 'set_parameter':
        if (block.params.parameter === 'exposure') {
          code.push(
            `${indentStr}client.set_exposure("${block.params.device}", ${block.params.value})`
          );
        } else if (block.params.parameter === 'wavelength') {
          code.push(
            `${indentStr}client.set_wavelength("${block.params.device}", ${block.params.value})`
          );
        } else {
          code.push(
            `${indentStr}client.set_parameter("${block.params.device}", "${block.params.parameter}", ${block.params.value})`
          );
        }
        break;

      case 'trigger':
        if (block.params.count > 1) {
          code.push(
            `${indentStr}for _ in range(${block.params.count}):`
          );
          code.push(
            `${indentStr}    client.trigger("${block.params.device}")`
          );
        } else {
          code.push(
            `${indentStr}client.trigger("${block.params.device}")`
          );
        }
        break;

      case 'read':
        code.push(
          `${indentStr}${block.params.variable} = client.read("${block.params.device}")`
        );
        break;

      case 'loop':
        code.push(
          `${indentStr}for ${block.params.variable} in range(${block.params.iterations}):`
        );
        if (block.children) {
          for (const child of block.children) {
            code.push(...generateActionCode(child, indent + 1));
          }
        } else {
          code.push(`${indentStr}    pass`);
        }
        break;

      case 'delay':
        code.push(`${indentStr}import time`);
        code.push(
          `${indentStr}time.sleep(${block.params.duration})`
        );
        break;

      case 'parallel':
        code.push(`${indentStr}# Parallel execution`);
        code.push(`${indentStr}import threading`);
        code.push(`${indentStr}threads = []`);
        if (block.children) {
          for (const child of block.children) {
            code.push(`${indentStr}def task_${child.id.slice(0, 8)}():`);
            code.push(...generateActionCode(child, indent + 1));
            code.push(
              `${indentStr}threads.append(threading.Thread(target=task_${child.id.slice(0, 8)}))`
            );
          }
          code.push(`${indentStr}for t in threads:`);
          code.push(`${indentStr}    t.start()`);
          code.push(`${indentStr}for t in threads:`);
          code.push(`${indentStr}    t.join()`);
        }
        break;

      default:
        code.push(`${indentStr}# Unknown action: ${block.type}`);
    }

    return code;
  };

  for (const action of actions) {
    lines.push(...generateActionCode(action));
  }

  if (lines.length === 6) {
    lines.push('# No actions defined');
  }

  lines.push('');
  lines.push('# Disconnect');
  lines.push('client.disconnect()');

  return lines.join('\n');
}
