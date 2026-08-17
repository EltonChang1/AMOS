FROM python:3.12.11-slim-bookworm

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PYSPARK_PYTHON=/usr/local/bin/python \
    SPARK_LOCAL_IP=127.0.0.1

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
       openjdk-17-jre-headless \
       r-base-core \
       r-cran-jsonlite \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10002 amos-toolbox \
    && useradd --uid 10002 --gid 10002 --no-create-home --shell /usr/sbin/nologin amos-toolbox

WORKDIR /opt/amos-toolbox
COPY toolbox/requirements.txt ./requirements.txt
RUN pip install --no-cache-dir --requirement requirements.txt

COPY toolbox/server.py toolbox/r_statistics.R ./

USER 10002:10002
EXPOSE 9000
ENTRYPOINT ["python", "/opt/amos-toolbox/server.py"]
