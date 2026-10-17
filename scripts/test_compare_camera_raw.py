"""Regression tests for the overlay auditor's coverage and registration guarantees."""
import unittest
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from PIL import Image
from compare_camera_raw import audit_elements
from capture_camera_raw import ordered_panels


class ElementAuditTests(unittest.TestCase):
    def test_wrong_reference_hash_is_rejected_before_writing_overlay_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.png"
            Image.new("RGB", (40, 40), "black").save(source)
            specification = root / "regions.json"
            specification.write_text(json.dumps({"sourceSha256": "0" * 64, "elements": []}), encoding="utf-8")
            output = root / "output"
            result = subprocess.run([sys.executable, str(Path(__file__).with_name("compare_camera_raw.py")),
                                     "--source", str(source), "--candidate", str(source),
                                     "--source-rect", "0", "0", "40", "40",
                                     "--candidate-rect", "0", "0", "40", "40",
                                     "--width", "40", "--regions", str(specification), "--output", str(output)],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("SHA256 mismatch", result.stderr)
            self.assertFalse(output.exists())

    def test_capture_order_keeps_the_sections_following_a_scroll_target_together(self):
        order = ["Light", "Color", "Curve", "Color Mixer", "Optics", "Lens Blur", "Calibration"]
        self.assertEqual(ordered_panels(order, "Optics"), ["Optics", "Lens Blur", "Calibration", "Light", "Color", "Curve", "Color Mixer"])
        self.assertEqual(ordered_panels(order, None), order)
        self.assertEqual(order[0], "Light")

    def test_missing_candidate_coverage_cannot_pass_even_when_padding_is_black(self):
        source = Image.new("RGB", (100, 100), "black")
        candidate = Image.new("RGB", (100, 50), "black")
        audit = audit_elements(source, candidate, {"elements": [{"id": "footer", "rect": [0, 40, 100, 100]}]})
        self.assertFalse(audit["elements"][0]["candidateCoverageComplete"])
        self.assertFalse(audit["allListedElementsPixelIdentical"])

    def test_displaced_control_is_compared_at_its_original_registered_position(self):
        source = Image.new("RGB", (100, 100), "black")
        source.paste("white", (10, 10, 20, 20))
        candidate = Image.new("RGB", (100, 100), "black")
        candidate.paste("white", (12, 10, 22, 20))
        audit = audit_elements(source, candidate, {"elements": [{"id": "control", "rect": [5, 5, 30, 30]}]})
        self.assertFalse(audit["allListedElementsPixelIdentical"])
        self.assertGreater(audit["elements"][0]["meanAbsoluteRgbDifference"][0], 0)
        self.assertTrue(audit_elements(source, source, {"elements": [{"id": "control", "rect": [5, 5, 30, 30]}]})["allListedElementsPixelIdentical"])

    def test_duplicate_ids_outside_rectangles_and_output_path_names_are_rejected(self):
        image = Image.new("RGB", (100, 100))
        for elements in [
            [{"id": "../escape", "rect": [0, 0, 10, 10]}],
            [{"id": "header", "rect": [0, 0, 101, 10]}],
            [{"id": "header", "rect": [0, 0, 10, 10]}] * 2,
        ]:
            with self.assertRaises(ValueError):
                audit_elements(image, image, {"elements": elements})

    def test_coordinates_cannot_be_reused_at_a_different_scale(self):
        image = Image.new("RGB", (270, 100))
        with self.assertRaises(ValueError):
            audit_elements(image, image, {"normalizedWidth": 540, "elements": [{"id": "title", "rect": [0, 0, 100, 20]}]})


if __name__ == "__main__":
    unittest.main()
