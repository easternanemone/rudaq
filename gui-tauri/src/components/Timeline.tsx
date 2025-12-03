import { ActionBlock } from '../types/experiment';
import { Droppable } from 'react-beautiful-dnd';
import ActionBlockComponent from './ActionBlockComponent';
import { ValidationIssue } from '../types/validation';

interface TimelineProps {
  actions: ActionBlock[];
  selectedBlock: ActionBlock | null;
  onSelectBlock: (block: ActionBlock) => void;
  onDeleteBlock: (path: number[]) => void;
  validationIssues?: ValidationIssue[];
}

function Timeline({
  actions,
  selectedBlock,
  onSelectBlock,
  onDeleteBlock,
  validationIssues = [],
}: TimelineProps) {
  return (
    <div className="h-full flex flex-col bg-slate-800">
      <div className="px-4 py-3 border-b border-slate-700">
        <h3 className="font-semibold text-white">Timeline</h3>
        <p className="text-xs text-slate-400 mt-1">
          {actions.length} action{actions.length !== 1 ? 's' : ''}
        </p>
      </div>

      <Droppable droppableId="timeline" type="ACTION">
        {(provided, snapshot) => (
          <div
            ref={provided.innerRef}
            {...provided.droppableProps}
            className={`
              flex-1 overflow-y-auto p-4
              ${
                snapshot.isDraggingOver
                  ? 'bg-blue-900/20 border-2 border-blue-400 border-dashed'
                  : ''
              }
            `}
          >
            {actions.length === 0 ? (
              <div className="h-full flex items-center justify-center">
                <div className="text-center text-slate-500">
                  <div className="text-4xl mb-2">👈</div>
                  <p className="text-sm">Drag actions from the palette</p>
                  <p className="text-xs mt-1">or click an action to add it</p>
                </div>
              </div>
            ) : (
              <div className="space-y-0">
                {actions.map((block, index) => (
                  <ActionBlockComponent
                    key={block.id}
                    block={block}
                    index={index}
                    path={[index]}
                    isSelected={selectedBlock?.id === block.id}
                    onSelect={onSelectBlock}
                    onDelete={onDeleteBlock}
                    validationIssues={validationIssues}
                  />
                ))}
              </div>
            )}
            {provided.placeholder}
          </div>
        )}
      </Droppable>
    </div>
  );
}

export default Timeline;
