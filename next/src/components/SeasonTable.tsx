export type SeasonRow = {
  season: string;
  operation: string;
  event?: string;
  version?: string;
  build: string;
};

function groupBuilds(rows: SeasonRow[]) {
  const groups = new Map<string, SeasonRow & { builds: string[] }>();
  for (const row of rows) {
    const key = [row.season, row.operation, row.event, row.version].join("|");
    const group = groups.get(key);
    if (group) group.builds.push(row.build);
    else groups.set(key, { ...row, builds: [row.build] });
  }
  return [...groups.values()];
}

export function BuildChips({ builds }: { builds: string[] }) {
  return (
    <span className="flex flex-col items-center gap-1">
      {builds.map((build) => (
        <code key={build} className="code-chip">
          {build}
        </code>
      ))}
    </span>
  );
}

export function SeasonTable({
  rows,
  showEvent,
  showVersion,
}: {
  rows: SeasonRow[];
  showEvent?: boolean;
  showVersion?: boolean;
}) {
  return (
    <div className="mb-5 w-fit max-w-full overflow-x-auto rounded-lg border border-border">
      <table className="season-table">
        <thead>
          <tr>
            <th>Season</th>
            <th>Operation</th>
            {showEvent ? <th>Event</th> : null}
            {showVersion ? <th>Version</th> : null}
            <th>Build</th>
          </tr>
        </thead>
        <tbody>
          {groupBuilds(rows).map(
            ({ season, operation, event, version, builds }) => (
              <tr key={builds[0]}>
                <td>{season}</td>
                <td>{operation}</td>
                {showEvent ? <td>{event}</td> : null}
                {showVersion ? <td>{version}</td> : null}
                <td>
                  <BuildChips builds={builds} />
                </td>
              </tr>
            ),
          )}
        </tbody>
      </table>
    </div>
  );
}
