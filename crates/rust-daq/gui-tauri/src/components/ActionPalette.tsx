import { ACTION_TEMPLATES, ActionTemplate } from '../types/experiment';
import { Draggable, Droppable } from 'react-beautiful-dnd';

interface ActionPaletteProps {
  onAddAction?: (template: ActionTemplate) => void;
}

function ActionPalette({ onAddAction }: ActionPaletteProps) {
  return (
    <div className="h-full flex flex-col bg-slate-800">
      <div className="px-4 py-3 border-b border-slate-700">
        <h3 className="font-semibold text-white">Action Palette</h3>
        <p className="text-xs text-slate-400 mt-1">
          Drag actions to the timeline
        </p>
      </div>

      <Droppable droppableId="palette" isDropDisabled={true}>
        {(provided) => (
          <div
            ref={provided.innerRef}
            {...provided.droppableProps}
            className="flex-1 overflow-y-auto p-4 space-y-2"
          >
            {ACTION_TEMPLATES.map((template, index) => (
              <Draggable
                key={template.type}
                draggableId={`palette-${template.type}`}
                index={index}
              >
                {(provided, snapshot) => (
                  <>
                    <div
                      ref={provided.innerRef}
                      {...provided.draggableProps}
                      {...provided.dragHandleProps}
                      className={`
                        p-3 rounded-lg border-2 cursor-move transition-all
                        ${
                          snapshot.isDragging
                            ? 'border-blue-500 bg-blue-900/50 shadow-lg'
                            : 'border-slate-600 bg-slate-700 hover:border-slate-500 hover:bg-slate-650'
                        }
                      `}
                      onClick={() => onAddAction?.(template)}
                    >
                      <div className="flex items-center gap-2">
                        <span className="text-xl">{template.icon}</span>
                        <div className="flex-1 min-w-0">
                          <div className="font-medium text-sm text-white">
                            {template.label}
                          </div>
                          <div className="text-xs text-slate-400 truncate">
                            {template.description}
                          </div>
                        </div>
                      </div>
                    </div>
                    {snapshot.isDragging && (
                      <div className="p-3 rounded-lg border-2 border-slate-600 bg-slate-700">
                        <div className="flex items-center gap-2">
                          <span className="text-xl">{template.icon}</span>
                          <div className="flex-1 min-w-0">
                            <div className="font-medium text-sm text-white">
                              {template.label}
                            </div>
                            <div className="text-xs text-slate-400 truncate">
                              {template.description}
                            </div>
                          </div>
                        </div>
                      </div>
                    )}
                  </>
                )}
              </Draggable>
            ))}
            {provided.placeholder}
          </div>
        )}
      </Droppable>
    </div>
  );
}

export default ActionPalette;
