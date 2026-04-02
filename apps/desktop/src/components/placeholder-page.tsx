type PlaceholderProps = {
  title: string;
  description: string;
};

export function PlaceholderPage({ title, description }: PlaceholderProps) {
  return (
    <section className="space-y-2">
      <h2 className="text-2xl font-semibold">{title}</h2>
      <p className="text-sm text-slate-300">{description}</p>
    </section>
  );
}