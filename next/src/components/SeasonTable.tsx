export type SeasonRow = {
  season: string;
  operation: string;
  event?: string;
  version?: string;
  build: string;
};

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
          {rows.map(({ season, operation, event, version, build }) => (
            <tr key={build}>
              <td>{season}</td>
              <td>{operation}</td>
              {showEvent ? (
                <td>
                  {event?.split(" / ").map((name) => (
                    <span key={name} className="block">
                      {name}
                    </span>
                  ))}
                </td>
              ) : null}
              {showVersion ? <td>{version}</td> : null}
              <td>
                <span className="flex flex-col items-center">
                  <code className="code-chip">{build}</code>
                </span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
