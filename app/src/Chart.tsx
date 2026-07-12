import { memo } from "react";
import {
  ResponsiveContainer, BarChart, Bar, LineChart, Line, AreaChart, Area,
  PieChart, Pie, Cell, XAxis, YAxis, CartesianGrid, Tooltip, Legend,
} from "recharts";

type Row = Record<string, string | number>;
type Spec = { type?: string; title?: string; data?: Row[]; series?: string[] };

const COLORS = ["#7c5cff", "#e89b73", "#8fc9a6", "#4a9e92", "#d99a5b", "#e0463a", "#5aa0e0", "#c77dff"];
const AXIS = { stroke: "var(--mut)", fontSize: 12 };

// Renders a chart from a ```chart JSON block emitted by Liara.
// memo: digitare nel prompt fa re-render dell'app → SENZA memo il grafico si ri-renderizzava e recharts
// ri-animava ("le candele rinascono" a ogni lettera). Con memo su `raw` invariato, il grafico resta fermo.
export const ChartView = memo(function ChartView({ raw }: { raw: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw);
  } catch {
    return <pre>{raw}</pre>;
  }
  const data = Array.isArray(spec.data) ? spec.data : [];
  if (data.length === 0) return <pre>{raw}</pre>;

  const type = (spec.type || "bar").toLowerCase();
  const series =
    spec.series && spec.series.length
      ? spec.series
      : Object.keys(data[0]).filter((k) => k !== "name" && typeof data[0][k] === "number");

  const grid = <CartesianGrid strokeDasharray="3 3" stroke="var(--line)" />;
  const common = (
    <>
      {grid}
      <XAxis dataKey="name" tick={AXIS} />
      <YAxis tick={AXIS} />
      <Tooltip contentStyle={{ background: "var(--panel)", border: "1px solid var(--line)", borderRadius: 8, color: "var(--txt)" }} />
      {series.length > 1 && <Legend />}
    </>
  );

  let chart: React.ReactElement;
  if (type === "line") {
    chart = (
      <LineChart data={data}>
        {common}
        {series.map((s, i) => <Line key={s} type="monotone" dataKey={s} stroke={COLORS[i % COLORS.length]} strokeWidth={2} dot={false} />)}
      </LineChart>
    );
  } else if (type === "area") {
    chart = (
      <AreaChart data={data}>
        {common}
        {series.map((s, i) => <Area key={s} type="monotone" dataKey={s} stroke={COLORS[i % COLORS.length]} fill={COLORS[i % COLORS.length]} fillOpacity={0.25} />)}
      </AreaChart>
    );
  } else if (type === "pie") {
    const key = series[0] || "value";
    chart = (
      <PieChart>
        <Tooltip contentStyle={{ background: "var(--panel)", border: "1px solid var(--line)", borderRadius: 8, color: "var(--txt)" }} />
        <Pie data={data} dataKey={key} nameKey="name" cx="50%" cy="46%" outerRadius="80%"
          label={(e: { name?: string; value?: number }) => `${e.name}: ${e.value}`}>
          {data.map((_, i) => <Cell key={i} fill={COLORS[i % COLORS.length]} />)}
        </Pie>
        <Legend layout="horizontal" verticalAlign="bottom" />
      </PieChart>
    );
  } else {
    chart = (
      <BarChart data={data}>
        {common}
        {series.map((s, i) => <Bar key={s} dataKey={s} fill={COLORS[i % COLORS.length]} radius={[4, 4, 0, 0]} />)}
      </BarChart>
    );
  }

  return (
    <div className="chartbox">
      {spec.title && <div className="charttitle">{spec.title}</div>}
      <ResponsiveContainer width="100%" height={type === "pie" ? 420 : 300}>
        {chart}
      </ResponsiveContainer>
    </div>
  );
});
