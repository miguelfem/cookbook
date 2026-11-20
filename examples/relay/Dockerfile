FROM python:3.13-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt \
    && groupadd --gid 10001 relay \
    && useradd --uid 10001 --gid relay --no-create-home relay \
    && mkdir /data
COPY relay.py .

# The master prepares the runtime volume, then Gunicorn drops worker privileges.
CMD ["sh", "-c", "chown relay:relay /data && exec gunicorn --user relay --group relay --bind \"0.0.0.0:${PORT:-8080}\" --workers 2 --timeout 20 relay:app"]
