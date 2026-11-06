/** A country's flag from the world-flags sprite; the caller loads the sprite stylesheet. */
export default function CountryFlag(props: { code: string }) {
  return (
    <span class="f32">
      <span class={`flag ${props.code.toLowerCase()}`} aria-hidden="true" />
    </span>
  );
}
