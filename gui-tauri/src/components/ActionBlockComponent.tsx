import { ActionBlock, ACTION_TEMPLATES } from '../types/experiment';
import { Draggable, Droppable } from 'react-beautiful-dnd';
import { ChevronDown, ChevronRight, Trash2, AlertCircle, AlertTriangle } from 'lucide-react';
import { useState } from 'react';
import { ValidationIssue } from '../types/validation';

interface ActionBlockComponentProps {
  block: ActionBlock;
  index: number;
  path: number[];
  isSelected: boolean;
  onSelect: (block: ActionBlock) => void;
  onDelete: (path: number[]) => void;
  validationIssues?: ValidationIssue[];
}

function ActionBlockComponent({
  block,
  index,
  path,
  isSelected,
  onSelect,
  onDelete,
  validationIssues = [],
}: ActionBlockComponentProps) {
  const [isExpanded, setIsExpanded] = useState(true);
  const template = ACTION_TEMPLATES.find((t) => t.type === block.type);
  const hasChildren = block.children && block.children.length > 0;
  const canHaveChildren = template?.canHaveChildren || false;

  // Find validation issues for this block
  const blockIssues = validationIssues.filter((issue) => issue.actionId === block.id);
  const hasErrors = blockIssues.some((issue) => issue.severity === 'error');
  const hasWarnings = blockIssues.some((issue) => issue.severity === 'warning');

  const getBlockColor = (type: string) => {
    // Override with validation colors if there are issues
    if (hasErrors) {
      return 'border-red-500 bg-red-900/20';
    }
    if (hasWarnings) {
      return 'border-yellow-500 bg-yellow-900/20';
    }

    const colors: Record<string, string> = {
      move_absolute: 'border-green-500 bg-green-900/20',
      move_relative: 'border-green-400 bg-green-900/20',
      set_parameter: 'border-yellow-500 bg-yellow-900/20',
      trigger: 'border-purple-500 bg-purple-900/20',
      read: 'border-blue-500 bg-blue-900/20',
      loop: 'border-orange-500 bg-orange-900/20',
      delay: 'border-gray-500 bg-gray-900/20',
      parallel: 'border-pink-500 bg-pink-900/20',
    };
    return colors[type] || 'border-slate-500 bg-slate-900/20';
  };

  const formatParams = (params: Record<string, any>) => {
    return Object.entries(params)
      .filter(([, value]) => value !== undefined && value !== '')
      .map(([key, value]) => `${key}: ${value}`)
      .join(', ');
  };

  return (
    <Draggable draggableId={block.id} index={index}>
      {(provided, snapshot) => (
        <div
          ref={provided.innerRef}
          {...provided.draggableProps}
          className={`
            mb-2 rounded-lg border-2 transition-all
            ${getBlockColor(block.type)}
            ${
              isSelected
                ? 'ring-2 ring-blue-400'
                : 'hover:ring-1 hover:ring-slate-500'
            }
            ${snapshot.isDragging ? 'shadow-xl opacity-80' : ''}
          `}
        >
          <div
            {...provided.dragHandleProps}
            className="p-3 cursor-move"
            onClick={() => onSelect(block)}
          >
            <div className="flex items-start gap-2">
              {canHaveChildren && (
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    setIsExpanded(!isExpanded);
                  }}
                  className="mt-0.5 text-slate-400 hover:text-white transition-colors"
                >
                  {isExpanded ? (
                    <ChevronDown size={16} />
                  ) : (
                    <ChevronRight size={16} />
                  )}
                </button>
              )}
              <span className="text-xl">{template?.icon}</span>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-white">
                    {template?.label}
                  </span>
                  {hasChildren && (
                    <span className="text-xs text-slate-400">
                      ({block.children!.length} action
                      {block.children!.length !== 1 ? 's' : ''})
                    </span>
                  )}
                  {/* Validation badges */}
                  {hasErrors && (
                    <div className="flex items-center gap-1 text-xs text-red-400" title="Has errors">
                      <AlertCircle size={14} />
                      <span>{blockIssues.filter(i => i.severity === 'error').length}</span>
                    </div>
                  )}
                  {hasWarnings && !hasErrors && (
                    <div className="flex items-center gap-1 text-xs text-yellow-400" title="Has warnings">
                      <AlertTriangle size={14} />
                      <span>{blockIssues.filter(i => i.severity === 'warning').length}</span>
                    </div>
                  )}
                </div>
                <div className="text-xs text-slate-400 mt-0.5 truncate">
                  {formatParams(block.params)}
                </div>
                {/* Show first validation issue as tooltip */}
                {blockIssues.length > 0 && (
                  <div className="text-xs mt-1 text-red-300">
                    {blockIssues[0].message}
                  </div>
                )}
              </div>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onDelete(path);
                }}
                className="text-slate-400 hover:text-red-400 transition-colors"
              >
                <Trash2 size={16} />
              </button>
            </div>
          </div>

          {/* Children container */}
          {canHaveChildren && isExpanded && (
            <Droppable droppableId={block.id} type="ACTION">
              {(provided, snapshot) => (
                <div
                  ref={provided.innerRef}
                  {...provided.droppableProps}
                  className={`
                    ml-8 mr-3 mb-3 p-2 rounded border border-dashed
                    ${
                      snapshot.isDraggingOver
                        ? 'border-blue-400 bg-blue-900/20'
                        : 'border-slate-600 bg-slate-900/20'
                    }
                    ${hasChildren ? '' : 'min-h-[40px]'}
                  `}
                >
                  {hasChildren ? (
                    block.children!.map((child, childIndex) => (
                      <ActionBlockComponent
                        key={child.id}
                        block={child}
                        index={childIndex}
                        path={[...path, childIndex]}
                        isSelected={isSelected && path[path.length - 1] === childIndex}
                        onSelect={onSelect}
                        onDelete={onDelete}
                        validationIssues={validationIssues}
                      />
                    ))
                  ) : (
                    <div className="text-xs text-slate-500 text-center py-2">
                      Drop actions here
                    </div>
                  )}
                  {provided.placeholder}
                </div>
              )}
            </Droppable>
          )}
        </div>
      )}
    </Draggable>
  );
}

export default ActionBlockComponent;
