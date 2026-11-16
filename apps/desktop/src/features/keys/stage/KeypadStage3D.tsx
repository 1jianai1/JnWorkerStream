import { useEffect, useRef, useState } from "react";
import type * as THREE_NS from "three";
import type { KeypadStageProps } from "./types";
import { keyLabel } from "./types";
import { useI18n } from "../../../lib/i18n";

interface KeyRuntime {
  mesh: THREE_NS.Mesh;
  edge: THREE_NS.LineSegments;
  label: HTMLButtonElement;
  css: { element: HTMLElement };
}

function cssToken(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function KeypadStage3D({ layout, visuals, selected, onSelect, describeKey, onUnavailable }: KeypadStageProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const keysRef = useRef<Map<number, KeyRuntime>>(new Map());
  const applyRef = useRef<(() => void) | null>(null);
  const invalidateRef = useRef<(() => void) | null>(null);
  const selectedRef = useRef(selected);
  const visualsRef = useRef(visuals);
  const onSelectRef = useRef(onSelect);
  const describeRef = useRef(describeKey);
  const [failed, setFailed] = useState(false);
  const { locale, t } = useI18n();
  const localeRef = useRef(locale);
  const visualsKey = JSON.stringify(visuals);

  useEffect(() => {
    selectedRef.current = selected;
    localeRef.current = locale;
    applyRef.current?.();
    invalidateRef.current?.();
  }, [selected, locale]);

  useEffect(() => {
    visualsRef.current = visuals;
    onSelectRef.current = onSelect;
    describeRef.current = describeKey;
    applyRef.current?.();
    invalidateRef.current?.();
  }, [visuals, onSelect, describeKey]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let disposed = false;
    let cleanup: (() => void) | undefined;

    (async () => {
      try {
        const THREE = await import("three");
        const { RoundedBoxGeometry } = await import("three/examples/jsm/geometries/RoundedBoxGeometry.js");
        const { CSS2DObject, CSS2DRenderer } = await import("three/examples/jsm/renderers/CSS2DRenderer.js");
        if (disposed) return;

        const width = host.clientWidth || 640;
        const height = host.clientHeight || 420;
        const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
        renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
        renderer.setSize(width, height);
        renderer.domElement.className = "keypad-canvas";
        renderer.domElement.setAttribute("aria-hidden", "true");
        host.appendChild(renderer.domElement);

        const labelRenderer = new CSS2DRenderer();
        labelRenderer.setSize(width, height);
        labelRenderer.domElement.className = "keypad-label-layer";
        host.appendChild(labelRenderer.domElement);

        const scene = new THREE.Scene();
        const viewSize = 3.6;
        const aspect = width / height;
        const camera = new THREE.OrthographicCamera(
          -viewSize * aspect,
          viewSize * aspect,
          viewSize,
          -viewSize,
          0.1,
          100,
        );
        camera.position.set(0, 9.4, 8.6);
        camera.lookAt(0, 0.2, 0);

        scene.add(new THREE.AmbientLight(0xffffff, 1.35));
        const light = new THREE.DirectionalLight(0xffffff, 1.9);
        light.position.set(4, 9, 6);
        scene.add(light);

        const colors = {
          card: cssToken("--jn-color-surface-card"),
          soft: cssToken("--jn-color-surface-soft"),
          hairline: cssToken("--jn-color-hairline"),
          info: cssToken("--jn-color-info"),
          body: cssToken("--jn-color-body"),
          ink: cssToken("--jn-color-ink"),
        };

        const baseGeometry = new RoundedBoxGeometry(7.4, 0.26, 7.4, 4, 0.16);
        const baseMaterial = new THREE.MeshLambertMaterial({ color: new THREE.Color(colors.soft) });
        const base = new THREE.Mesh(baseGeometry, baseMaterial);
        base.position.y = -0.13;
        scene.add(base);
        const baseEdges = new THREE.LineSegments(
          new THREE.EdgesGeometry(baseGeometry, 30),
          new THREE.LineBasicMaterial({ color: new THREE.Color(colors.hairline) }),
        );
        baseEdges.position.y = -0.13;
        scene.add(baseEdges);

        const geometry = new RoundedBoxGeometry(1.34, 0.56, 1.34, 4, 0.11);
        const edgeGeometry = new THREE.EdgesGeometry(geometry, 30);
        const raycaster = new THREE.Raycaster();
        const pointer = new THREE.Vector2();
        let hovered: number | null = null;

        const apply = () => {
          const currentVisuals = visualsRef.current;
          for (const [keyId, runtime] of keysRef.current) {
            const visual = currentVisuals[keyId];
            const isSelected = selectedRef.current === keyId;
            const isHovered = hovered === keyId;
            runtime.edge.visible = isSelected || isHovered;
            const edgeMaterial = runtime.edge.material as THREE_NS.LineBasicMaterial;
            edgeMaterial.color.set(isSelected ? colors.info : colors.body);
            const meshMaterial = runtime.mesh.material as THREE_NS.MeshLambertMaterial;
            meshMaterial.emissive.set(colors.info);
            meshMaterial.emissiveIntensity = isSelected ? 0.16 : 0;
            runtime.mesh.parent?.position.setY(0.28 + (isSelected ? 0.06 : 0));
            runtime.label.className = `key3d-label${isSelected ? " is-selected" : ""}`;
            runtime.label.setAttribute("aria-pressed", String(isSelected));
            runtime.label.setAttribute("aria-label", describeRef.current(keyId));
            const text = runtime.label.querySelector<HTMLElement>(".key3d-label__text");
            if (text) text.textContent = keyLabel(layout, keyId, localeRef.current);
            const dot = runtime.label.querySelector<HTMLElement>(".key3d-label__dot");
            if (dot) {
              dot.hidden = !visual || !visual.configured || visual.route === "devicePassthrough";
              dot.className = `key3d-label__dot${visual?.route === "deviceMacro" ? " key3d-label__dot--macro" : " key3d-label__dot--host"}`;
            }
            const conflict = runtime.label.querySelector<HTMLElement>(".key3d-label__conflict");
            if (conflict) conflict.hidden = !visual?.conflict;
            const dirty = runtime.label.querySelector<HTMLElement>(".key3d-label__dirty");
            if (dirty) dirty.hidden = !visual?.dirty;
          }
        };
        applyRef.current = apply;

        for (const entry of layout.keys) {
          const group = new THREE.Group();
          group.name = `key_K${entry.key_id}`;
          group.position.set((entry.col - 1.5) * 1.52, 0.28, (entry.row - 1.5) * 1.52);
          const material = new THREE.MeshLambertMaterial({ color: new THREE.Color(colors.card) });
          const mesh = new THREE.Mesh(geometry, material);
          mesh.userData.keyId = entry.key_id;
          group.add(mesh);
          const edge = new THREE.LineSegments(
            edgeGeometry,
            new THREE.LineBasicMaterial({ color: new THREE.Color(colors.body), transparent: true, opacity: 0.9 }),
          );
          edge.visible = false;
          group.add(edge);
          scene.add(group);

          const label = document.createElement("button");
          label.type = "button";
          label.className = "key3d-label";
          label.dataset.testid = `key-K${entry.key_id}`;
          label.dataset.keyId = String(entry.key_id);
          label.innerHTML = [
            '<span class="key3d-label__text"></span>',
            '<span class="key3d-label__dot" hidden></span>',
            '<span class="key3d-label__conflict" hidden></span>',
            '<span class="key3d-label__dirty" hidden></span>',
          ].join("");
          label.addEventListener("click", () => onSelectRef.current(entry.key_id));
          const css = new CSS2DObject(label);
          css.position.set(0, 0.36, 0);
          css.center.set(0.5, 0.5);
          group.add(css);
          keysRef.current.set(entry.key_id, { mesh, edge, label, css });
        }

        const invalidate = () => {
          renderer.render(scene, camera);
          labelRenderer.render(scene, camera);
        };
        invalidateRef.current = invalidate;

        const pick = (event: PointerEvent): number | null => {
          const rect = renderer.domElement.getBoundingClientRect();
          pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1;
          pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1;
          raycaster.setFromCamera(pointer, camera);
          const meshes: THREE_NS.Mesh[] = [];
          for (const runtime of keysRef.current.values()) meshes.push(runtime.mesh);
          const hit = raycaster.intersectObjects(meshes, false)[0];
          return hit ? (hit.object.userData.keyId as number) : null;
        };

        const onPointerDown = (event: PointerEvent) => {
          const keyId = pick(event);
          if (keyId !== null) onSelectRef.current(keyId);
          else onSelectRef.current(null);
        };
        const onPointerMove = (event: PointerEvent) => {
          const keyId = pick(event);
          if (keyId !== hovered) {
            hovered = keyId;
            renderer.domElement.style.cursor = keyId !== null ? "pointer" : "default";
            apply();
            invalidate();
          }
        };
        const onPointerLeave = () => {
          if (hovered !== null) {
            hovered = null;
            apply();
            invalidate();
          }
        };
        renderer.domElement.addEventListener("pointerdown", onPointerDown);
        renderer.domElement.addEventListener("pointermove", onPointerMove);
        renderer.domElement.addEventListener("pointerleave", onPointerLeave);

        const resize = () => {
          const nextWidth = host.clientWidth || width;
          const nextHeight = host.clientHeight || height;
          const nextAspect = nextWidth / Math.max(1, nextHeight);
          camera.left = -viewSize * nextAspect;
          camera.right = viewSize * nextAspect;
          camera.top = viewSize;
          camera.bottom = -viewSize;
          camera.updateProjectionMatrix();
          renderer.setSize(nextWidth, nextHeight);
          labelRenderer.setSize(nextWidth, nextHeight);
          invalidate();
        };
        const observer = new ResizeObserver(resize);
        observer.observe(host);

        apply();
        invalidate();

        cleanup = () => {
          observer.disconnect();
          renderer.domElement.removeEventListener("pointerdown", onPointerDown);
          renderer.domElement.removeEventListener("pointermove", onPointerMove);
          renderer.domElement.removeEventListener("pointerleave", onPointerLeave);
          geometry.dispose();
          edgeGeometry.dispose();
          baseGeometry.dispose();
          baseMaterial.dispose();
          for (const runtime of keysRef.current.values()) {
            (runtime.mesh.material as THREE_NS.Material).dispose();
            (runtime.edge.material as THREE_NS.Material).dispose();
            runtime.css.element.remove();
          }
          keysRef.current.clear();
          renderer.dispose();
          renderer.domElement.remove();
          labelRenderer.domElement.remove();
          applyRef.current = null;
          invalidateRef.current = null;
        };
      } catch (error) {
        console.warn("[keypad] 3D unavailable, falling back to 2D", error);
        if (!disposed) {
          setFailed(true);
          onUnavailable?.();
        }
      }
    })();

    return () => {
      disposed = true;
      cleanup?.();
    };
    // The scene is created once; visual updates flow through refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    applyRef.current?.();
    invalidateRef.current?.();
  }, [visualsKey, locale]);

  if (failed) {
    return (
      <div className="keypad-stage keypad-stage--failed" role="status">
        <span>{t("keys.stage.switched")}</span>
      </div>
    );
  }

  return <div className="keypad-stage keypad-stage--3d" ref={hostRef} />;
}
