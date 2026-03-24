import { LineChart, Line } from 'recharts';

export default function Sparkline({ data = [], color = '#f59e0b', width = 40, height = 16 }) {
  const chartData = data.length > 0
    ? data.map((v, i) => ({ i, v }))
    : [{ i: 0, v: 0 }, { i: 1, v: 0 }];

  return (
    <div className="shrink-0" style={{ width, height }}>
      <LineChart width={width} height={height} data={chartData}>
        <Line
          type="monotone"
          dataKey="v"
          stroke={color}
          strokeWidth={1.5}
          dot={false}
          isAnimationActive={false}
        />
      </LineChart>
    </div>
  );
}
