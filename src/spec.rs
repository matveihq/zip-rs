use crate::result::{ZipError, ZipResult};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io;
use std::io::prelude::*;

#[cfg(feature = "async")]
use crate::async_util::Compat;
#[cfg(feature = "async")]
use crate::async_util::CompatExt;
#[cfg(feature = "async")]
use futures::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite};
#[cfg(feature = "async")]
use std::pin::Pin;
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt as TokioAsyncReadExt, AsyncWriteExt as TokioAsyncWriteExt};

pub const LOCAL_FILE_HEADER_SIGNATURE: u32 = 0x04034b50;
pub const CENTRAL_DIRECTORY_HEADER_SIGNATURE: u32 = 0x02014b50;
const CENTRAL_DIRECTORY_END_SIGNATURE: u32 = 0x06054b50;
pub const ZIP64_CENTRAL_DIRECTORY_END_SIGNATURE: u32 = 0x06064b50;
const ZIP64_CENTRAL_DIRECTORY_END_LOCATOR_SIGNATURE: u32 = 0x07064b50;

pub struct CentralDirectoryEnd {
    pub disk_number: u16,
    pub disk_with_central_directory: u16,
    pub number_of_files_on_this_disk: u16,
    pub number_of_files: u16,
    pub central_directory_size: u32,
    pub central_directory_offset: u32,
    pub zip_file_comment: Vec<u8>,
}

impl CentralDirectoryEnd {
    pub fn parse<T: Read>(reader: &mut T) -> ZipResult<CentralDirectoryEnd> {
        let magic = reader.read_u32::<LittleEndian>()?;
        if magic != CENTRAL_DIRECTORY_END_SIGNATURE {
            return Err(ZipError::InvalidArchive("Invalid digital signature header"));
        }
        let disk_number = reader.read_u16::<LittleEndian>()?;
        let disk_with_central_directory = reader.read_u16::<LittleEndian>()?;
        let number_of_files_on_this_disk = reader.read_u16::<LittleEndian>()?;
        let number_of_files = reader.read_u16::<LittleEndian>()?;
        let central_directory_size = reader.read_u32::<LittleEndian>()?;
        let central_directory_offset = reader.read_u32::<LittleEndian>()?;
        let zip_file_comment_length = reader.read_u16::<LittleEndian>()? as usize;
        let mut zip_file_comment = vec![0; zip_file_comment_length];
        reader.read_exact(&mut zip_file_comment)?;

        Ok(CentralDirectoryEnd {
            disk_number,
            disk_with_central_directory,
            number_of_files_on_this_disk,
            number_of_files,
            central_directory_size,
            central_directory_offset,
            zip_file_comment,
        })
    }

    #[cfg(feature = "async")]
    pub async fn parse_async<T: AsyncRead>(reader: Pin<&mut T>) -> ZipResult<CentralDirectoryEnd> {
        let mut reader = reader.compat();
        let magic = reader.read_u32_le().await?;
        if magic != CENTRAL_DIRECTORY_END_SIGNATURE {
            return Err(ZipError::InvalidArchive("Invalid digital signature header"));
        }
        let disk_number = reader.read_u16_le().await?;
        let disk_with_central_directory = reader.read_u16_le().await?;
        let number_of_files_on_this_disk = reader.read_u16_le().await?;
        let number_of_files = reader.read_u16_le().await?;
        let central_directory_size = reader.read_u32_le().await?;
        let central_directory_offset = reader.read_u32_le().await?;
        let zip_file_comment_length = reader.read_u16_le().await? as usize;
        let mut zip_file_comment = vec![0; zip_file_comment_length];

        let mut reader = reader.into_inner();
        reader.read_exact(&mut zip_file_comment).await?;

        Ok(CentralDirectoryEnd {
            disk_number,
            disk_with_central_directory,
            number_of_files_on_this_disk,
            number_of_files,
            central_directory_size,
            central_directory_offset,
            zip_file_comment,
        })
    }

    pub fn find_and_parse<T: Read + io::Seek>(
        reader: &mut T,
    ) -> ZipResult<(CentralDirectoryEnd, u64)> {
        const HEADER_SIZE: u64 = 22;
        const BYTES_BETWEEN_MAGIC_AND_COMMENT_SIZE: u64 = HEADER_SIZE - 6;
        let file_length = reader.seek(io::SeekFrom::End(0))?;

        let search_upper_bound = file_length.saturating_sub(HEADER_SIZE + ::std::u16::MAX as u64);

        if file_length < HEADER_SIZE {
            return Err(ZipError::InvalidArchive("Invalid zip header"));
        }

        let mut pos = file_length - HEADER_SIZE;
        while pos >= search_upper_bound {
            reader.seek(io::SeekFrom::Start(pos as u64))?;
            if reader.read_u32::<LittleEndian>()? == CENTRAL_DIRECTORY_END_SIGNATURE {
                reader.seek(io::SeekFrom::Current(
                    BYTES_BETWEEN_MAGIC_AND_COMMENT_SIZE as i64,
                ))?;
                let cde_start_pos = reader.seek(io::SeekFrom::Start(pos as u64))?;
                return CentralDirectoryEnd::parse(reader).map(|cde| (cde, cde_start_pos));
            }
            pos = match pos.checked_sub(1) {
                Some(p) => p,
                None => break,
            };
        }
        Err(ZipError::InvalidArchive(
            "Could not find central directory end",
        ))
    }

    #[cfg(feature = "async")]
    pub async fn find_and_parse_async<T: AsyncRead + AsyncSeek>(
        mut reader: Pin<&mut T>,
    ) -> ZipResult<(CentralDirectoryEnd, u64)> {
        const HEADER_SIZE: u64 = 22;
        const BYTES_BETWEEN_MAGIC_AND_COMMENT_SIZE: u64 = HEADER_SIZE - 6;
        let file_length = reader.seek(io::SeekFrom::End(0)).await?;

        let search_upper_bound = file_length.saturating_sub(HEADER_SIZE + ::std::u16::MAX as u64);

        if file_length < HEADER_SIZE {
            return Err(ZipError::InvalidArchive("Invalid zip header"));
        }

        let mut pos = file_length - HEADER_SIZE;
        while pos >= search_upper_bound {
            reader.seek(io::SeekFrom::Start(pos as u64)).await?;
            if reader.compat_mut().read_u32_le().await.expect("fff")
                == CENTRAL_DIRECTORY_END_SIGNATURE
            {
                reader
                    .seek(io::SeekFrom::Current(
                        BYTES_BETWEEN_MAGIC_AND_COMMENT_SIZE as i64,
                    ))
                    .await?;
                let comment_length = reader.compat_mut().read_u16_le().await? as u64;
                if file_length - pos - HEADER_SIZE == comment_length {
                    let cde_start_pos = reader.seek(io::SeekFrom::Start(pos as u64)).await?;
                    return CentralDirectoryEnd::parse_async(reader)
                        .await
                        .map(|cde| (cde, cde_start_pos));
                }
            }
            pos = match pos.checked_sub(1) {
                Some(p) => p,
                None => break,
            };
        }
        Err(ZipError::InvalidArchive(
            "Could not find central directory end",
        ))
    }

    pub fn write<T: Write>(&self, writer: &mut T) -> ZipResult<()> {
        writer.write_u32::<LittleEndian>(CENTRAL_DIRECTORY_END_SIGNATURE)?;
        writer.write_u16::<LittleEndian>(self.disk_number)?;
        writer.write_u16::<LittleEndian>(self.disk_with_central_directory)?;
        writer.write_u16::<LittleEndian>(self.number_of_files_on_this_disk)?;
        writer.write_u16::<LittleEndian>(self.number_of_files)?;
        writer.write_u32::<LittleEndian>(self.central_directory_size)?;
        writer.write_u32::<LittleEndian>(self.central_directory_offset)?;
        writer.write_u16::<LittleEndian>(self.zip_file_comment.len() as u16)?;
        writer.write_all(&self.zip_file_comment)?;
        Ok(())
    }

    #[cfg(feature = "async")]
    pub async fn write_async<T: AsyncWrite + Unpin>(&self, writer: &mut T) -> ZipResult<()> {
        let mut writer = Compat(writer);
        writer.write_u32_le(CENTRAL_DIRECTORY_END_SIGNATURE).await?;
        writer.write_u16_le(self.disk_number).await?;
        writer
            .write_u16_le(self.disk_with_central_directory)
            .await?;
        writer
            .write_u16_le(self.number_of_files_on_this_disk)
            .await?;
        writer.write_u16_le(self.number_of_files).await?;
        writer.write_u32_le(self.central_directory_size).await?;
        writer.write_u32_le(self.central_directory_offset).await?;
        writer
            .write_u16_le(self.zip_file_comment.len() as u16)
            .await?;
        writer.write_all(&self.zip_file_comment).await?;
        Ok(())
    }
}

pub struct Zip64CentralDirectoryEndLocator {
    pub disk_with_central_directory: u32,
    pub end_of_central_directory_offset: u64,
    pub number_of_disks: u32,
}

impl Zip64CentralDirectoryEndLocator {
    pub fn parse<T: Read>(reader: &mut T) -> ZipResult<Zip64CentralDirectoryEndLocator> {
        let magic = reader.read_u32::<LittleEndian>()?;
        if magic != ZIP64_CENTRAL_DIRECTORY_END_LOCATOR_SIGNATURE {
            return Err(ZipError::InvalidArchive(
                "Invalid zip64 locator digital signature header",
            ));
        }
        let disk_with_central_directory = reader.read_u32::<LittleEndian>()?;
        let end_of_central_directory_offset = reader.read_u64::<LittleEndian>()?;
        let number_of_disks = reader.read_u32::<LittleEndian>()?;

        Ok(Zip64CentralDirectoryEndLocator {
            disk_with_central_directory,
            end_of_central_directory_offset,
            number_of_disks,
        })
    }

    #[cfg(feature = "async")]
    pub async fn parse_async<T: AsyncRead>(
        mut reader: Pin<&mut T>,
    ) -> ZipResult<Zip64CentralDirectoryEndLocator> {
        let magic = reader.compat_mut().read_u32_le().await?;
        if magic != ZIP64_CENTRAL_DIRECTORY_END_LOCATOR_SIGNATURE {
            return Err(ZipError::InvalidArchive(
                "Invalid zip64 locator digital signature header",
            ));
        }
        let disk_with_central_directory = reader.compat_mut().read_u32_le().await?;
        let end_of_central_directory_offset = reader.compat_mut().read_u64_le().await?;
        let number_of_disks = reader.compat_mut().read_u32_le().await?;

        Ok(Zip64CentralDirectoryEndLocator {
            disk_with_central_directory,
            end_of_central_directory_offset,
            number_of_disks,
        })
    }
}

pub struct Zip64CentralDirectoryEnd {
    pub version_made_by: u16,
    pub version_needed_to_extract: u16,
    pub disk_number: u32,
    pub disk_with_central_directory: u32,
    pub number_of_files_on_this_disk: u64,
    pub number_of_files: u64,
    pub central_directory_size: u64,
    pub central_directory_offset: u64,
    //pub extensible_data_sector: Vec<u8>, <-- We don't do anything with this at the moment.
}

impl Zip64CentralDirectoryEnd {
    pub fn find_and_parse<T: Read + io::Seek>(
        reader: &mut T,
        nominal_offset: u64,
        search_upper_bound: u64,
    ) -> ZipResult<(Zip64CentralDirectoryEnd, u64)> {
        let mut pos = nominal_offset;

        while pos <= search_upper_bound {
            reader.seek(io::SeekFrom::Start(pos))?;

            if reader.read_u32::<LittleEndian>()? == ZIP64_CENTRAL_DIRECTORY_END_SIGNATURE {
                let archive_offset = pos - nominal_offset;

                let _record_size = reader.read_u64::<LittleEndian>()?;
                // We would use this value if we did anything with the "zip64 extensible data sector".

                let version_made_by = reader.read_u16::<LittleEndian>()?;
                let version_needed_to_extract = reader.read_u16::<LittleEndian>()?;
                let disk_number = reader.read_u32::<LittleEndian>()?;
                let disk_with_central_directory = reader.read_u32::<LittleEndian>()?;
                let number_of_files_on_this_disk = reader.read_u64::<LittleEndian>()?;
                let number_of_files = reader.read_u64::<LittleEndian>()?;
                let central_directory_size = reader.read_u64::<LittleEndian>()?;
                let central_directory_offset = reader.read_u64::<LittleEndian>()?;

                return Ok((
                    Zip64CentralDirectoryEnd {
                        version_made_by,
                        version_needed_to_extract,
                        disk_number,
                        disk_with_central_directory,
                        number_of_files_on_this_disk,
                        number_of_files,
                        central_directory_size,
                        central_directory_offset,
                    },
                    archive_offset,
                ));
            }

            pos += 1;
        }

        Err(ZipError::InvalidArchive(
            "Could not find ZIP64 central directory end",
        ))
    }

    #[cfg(feature = "async")]
    pub async fn find_and_parse_async<T: AsyncRead + AsyncSeek>(
        mut reader: Pin<&mut T>,
        nominal_offset: u64,
        search_upper_bound: u64,
    ) -> ZipResult<(Zip64CentralDirectoryEnd, u64)> {
        let mut pos = nominal_offset;

        while pos <= search_upper_bound {
            reader.seek(io::SeekFrom::Start(pos)).await?;

            if reader.compat_mut().read_u32_le().await? == ZIP64_CENTRAL_DIRECTORY_END_SIGNATURE {
                let archive_offset = pos - nominal_offset;
                let mut reader = reader.compat_mut();

                let _record_size = reader.read_u64_le().await?;
                // We would use this value if we did anything with the "zip64 extensible data sector".

                let version_made_by = reader.read_u16_le().await?;
                let version_needed_to_extract = reader.read_u16_le().await?;
                let disk_number = reader.read_u32_le().await?;
                let disk_with_central_directory = reader.read_u32_le().await?;
                let number_of_files_on_this_disk = reader.read_u64_le().await?;
                let number_of_files = reader.read_u64_le().await?;
                let central_directory_size = reader.read_u64_le().await?;
                let central_directory_offset = reader.read_u64_le().await?;

                return Ok((
                    Zip64CentralDirectoryEnd {
                        version_made_by,
                        version_needed_to_extract,
                        disk_number,
                        disk_with_central_directory,
                        number_of_files_on_this_disk,
                        number_of_files,
                        central_directory_size,
                        central_directory_offset,
                    },
                    archive_offset,
                ));
            }

            pos += 1;
        }

        Err(ZipError::InvalidArchive(
            "Could not find ZIP64 central directory end",
        ))
    }
}
