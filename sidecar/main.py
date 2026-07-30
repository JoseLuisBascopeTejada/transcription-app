"""FastAPI entry point for the transcription sidecar."""

from __future__ import annotations

import logging
import os
from contextlib import asynccontextmanager
from typing import Literal

from fastapi import FastAPI, Response
from fastapi.responses import JSONResponse
from pydantic import BaseModel

from models import warm_up
from transcribe import transcribe_chunk, transcribe_full
from diarize import diarize
from reconcile import reconcile

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s")
logger = logging.getLogger(__name__)

_ready: bool = False


@asynccontextmanager
async def lifespan(app: FastAPI):
    global _ready
    logger.info("Starting sidecar — loading Whisper model...")
    warm_up()
    _ready = True
    logger.info("Model ready. Accepting requests.")
    yield
    logger.info("Sidecar shutting down.")


app = FastAPI(title="Transcription Sidecar", lifespan=lifespan)


class TranscribeRequest(BaseModel):
    audio_path: str
    mode: Literal["live-chunk", "full-file"]
    language: str | None = None


class TranscribeResponse(BaseModel):
    segments: list[dict]


class DiarizeRequest(BaseModel):
    audio_path: str


class DiarizeResponse(BaseModel):
    turns: list[dict]


class ReconcileRequest(BaseModel):
    whisper_segments: list[dict]
    diarization_turns: list[dict]


class ReconcileResponse(BaseModel):
    segments: list[dict]


@app.get("/health")
def health(response: Response):
    if _ready:
        return {"status": "ready"}
    response.status_code = 503
    return {"status": "loading"}


@app.post("/transcribe", response_model=TranscribeResponse)
def transcribe(req: TranscribeRequest):
    if req.mode == "live-chunk":
        segments = transcribe_chunk(req.audio_path, language=req.language)
    else:
        segments = transcribe_full(req.audio_path, language=req.language)
    return TranscribeResponse(segments=segments)


@app.post("/diarize", response_model=DiarizeResponse)
def diarize_endpoint(req: DiarizeRequest):
    try:
        turns = diarize(req.audio_path)
    except Exception as exc:
        logger.error("Diarization failed: %s", exc)
        return JSONResponse(
            status_code=503,
            content={"error": f"Diarization unavailable: {exc}"},
        )
    return DiarizeResponse(turns=turns)


@app.post("/reconcile", response_model=ReconcileResponse)
def reconcile_endpoint(req: ReconcileRequest):
    try:
        segments = reconcile(req.whisper_segments, req.diarization_turns)
    except Exception as exc:
        logger.error("Reconciliation failed: %s", exc)
        return JSONResponse(
            status_code=500,
            content={"error": f"Reconciliation failed: {exc}"},
        )
    return ReconcileResponse(segments=segments)


if __name__ == "__main__":
    import uvicorn

    port = int(os.environ.get("SIDECAR_PORT", "8756"))
    uvicorn.run("main:app", host="127.0.0.1", port=port, log_level="info")
