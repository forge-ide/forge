import type { Component } from 'solid-js';
import { listSkills, SESSION_WIDE_SCOPE } from '../../ipc/catalog';
import { RosterPane } from './RosterPane';

export interface SkillsPaneProps {
  workspaceRoot: string | null;
}

/** Workspace Skills pane. Lists skills the workspace + user roster
 *  declare; the Catalog page is the management surface. */
export const SkillsPane: Component<SkillsPaneProps> = (props) => (
  <RosterPane
    title="SKILLS"
    slug="skills"
    workspaceRoot={props.workspaceRoot}
    fetcher={(ws) => listSkills(ws, SESSION_WIDE_SCOPE)}
    catalogRoute="/catalog/skills"
    emptyMessage="No skills available."
    rowFor={(entry) => {
      if (entry.entry.type !== 'Skill') return null;
      return { id: entry.entry.id };
    }}
  />
);
