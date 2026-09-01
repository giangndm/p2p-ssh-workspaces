use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, copy};

/// Read and write halves of one byte stream.
pub(crate) struct StreamParts<R, W> {
    pub(crate) reader: R,
    pub(crate) writer: W,
}

/// Copy bytes in both directions and propagate half-close.
pub(crate) async fn bridge<Lr, Lw, Rr, Rw>(
    left: StreamParts<Lr, Lw>,
    right: StreamParts<Rr, Rw>,
) -> std::io::Result<()>
where
    Lr: AsyncRead + Unpin + Send,
    Lw: AsyncWrite + Unpin + Send,
    Rr: AsyncRead + Unpin + Send,
    Rw: AsyncWrite + Unpin + Send,
{
    let StreamParts {
        reader: mut left_reader,
        writer: mut left_writer,
    } = left;
    let StreamParts {
        reader: mut right_reader,
        writer: mut right_writer,
    } = right;
    let left_to_right = async {
        copy(&mut left_reader, &mut right_writer).await?;
        right_writer.shutdown().await
    };
    let right_to_left = async {
        copy(&mut right_reader, &mut left_writer).await?;
        left_writer.shutdown().await
    };

    tokio::try_join!(left_to_right, right_to_left)?;
    Ok(())
}
